//! 带版本与字段标签的状态迁移。
//!
//! [`crate::schema`] 中的 schema 哈希用于回答“内存布局是否相同”。当结果为否时，
//! guest 仍需在重载后保留用户数据。单一的 `postcard` 二进制块无法做到这一点：
//! 在末尾新增一个字段会改变后续所有字节偏移，且缺失字段与损坏数据无法区分。
//!
//! 此处的信封（envelope）格式是能同时解决这两个问题的最小封装：
//!
//!
//!
//! ```text
//! "UBMF" | u16 version | u16 count | count × { u16 field_id, u32 len, len bytes }
//! ```
//!
//! * 各字段由 guest 逐一编码（均使用 `postcard`），从而可独立增删字段。
//! * [`migrate`] 会重排字段、丢弃未知字段，并用声明的默认值填充新字段。
//! * 缺失**必需**字段将返回错误，而非静默清零——host 可据此回退到初始状态并给出提示。
//! *
//!
//!
//!
//! 该格式刻意保持朴素：固定宽度的小端序头部与不透明负载，
//! 使 host 侧工具无需链接 guest 即可解析。
//!

use std::borrow::Cow;

/// 信封魔数，`UBMF`（"Unbound migration format"）。
pub const MAGIC: [u8; 4] = *b"UBMF";
/// 当前信封版本号。
pub const ENVELOPE_VERSION: u16 = 1;
const HEADER_LEN: usize = 8;
const RECORD_HEADER_LEN: usize = 6;

/// 单个已编码字段。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    /// 稳定的字段 ID。
    pub id: u16,
    /// 字段负载，对本 crate 不透明。
    pub bytes: &'a [u8],
}

impl<'a> Field<'a> {
    #[must_use]
    pub const fn new(id: u16, bytes: &'a [u8]) -> Self {
        Self { id, bytes }
    }
}

/// 解码或迁移失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationError {
    /// 信封长度小于头部声明的长度。
    Truncated,
    /// 魔数与 [`MAGIC`] 不匹配。
    BadMagic,
    /// 信封版本高于当前构建所能理解的版本。
    UnsupportedVersion(u16),
    /// 同一字段 ID 出现了两次。
    DuplicateField(u16),
    /// 最后一条记录之后仍有剩余字节。
    TrailingBytes,
    /// 记录数量超出 `u16` 头部表示范围。
    TooManyFields,
    /// 某条负载超出 `u32` 长度字段表示范围。
    FieldTooLarge,
    /// 负载中缺少必需字段。
    MissingField(u16),
}

impl core::fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("migration payload is truncated"),
            Self::BadMagic => formatter.write_str("migration payload has a bad magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported migration version {version}")
            }
            Self::DuplicateField(id) => write!(formatter, "duplicate field id {id}"),
            Self::TrailingBytes => formatter.write_str("trailing bytes after the last field"),
            Self::TooManyFields => formatter.write_str("too many fields"),
            Self::FieldTooLarge => formatter.write_str("field payload is too large"),
            Self::MissingField(id) => write!(formatter, "missing required field {id}"),
        }
    }
}

impl std::error::Error for MigrationError {}

/// 将字段列表编码为信封格式。
///
/// # Errors
/// 当负载无法由头部表示时，返回 [`MigrationError::TooManyFields`] 或
/// [`MigrationError::FieldTooLarge`]。
pub fn encode(fields: &[Field<'_>]) -> Result<Vec<u8>, MigrationError> {
    let count = u16::try_from(fields.len()).map_err(|_| MigrationError::TooManyFields)?;
    let total = fields.iter().try_fold(HEADER_LEN, |total, field| {
        let len = u32::try_from(field.bytes.len()).map_err(|_| MigrationError::FieldTooLarge)?;
        total
            .checked_add(RECORD_HEADER_LEN + len as usize)
            .ok_or(MigrationError::TooManyFields)
    })?;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&ENVELOPE_VERSION.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for field in fields {
        let len = u32::try_from(field.bytes.len()).map_err(|_| MigrationError::FieldTooLarge)?;
        out.extend_from_slice(&field.id.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(field.bytes);
    }
    Ok(out)
}

/// 在不解析具体负载的前提下解码信封。
///
/// # Errors
/// 当负载损坏、版本未知、字段 ID 重复或存在尾部多余字节时，返回 [`MigrationError`]。
///
pub fn decode(bytes: &[u8]) -> Result<Vec<Field<'_>>, MigrationError> {
    if bytes.len() < HEADER_LEN {
        return Err(MigrationError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(MigrationError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != ENVELOPE_VERSION {
        return Err(MigrationError::UnsupportedVersion(version));
    }
    let count = usize::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let mut offset = HEADER_LEN;
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        if bytes.len() - offset < RECORD_HEADER_LEN {
            return Err(MigrationError::Truncated);
        }
        let id = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let len = u32::from_le_bytes([
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
        ]) as usize;
        offset += RECORD_HEADER_LEN;
        let end = offset.checked_add(len).ok_or(MigrationError::Truncated)?;
        if end > bytes.len() {
            return Err(MigrationError::Truncated);
        }
        if fields.iter().any(|field: &Field<'_>| field.id == id) {
            return Err(MigrationError::DuplicateField(id));
        }
        fields.push(Field::new(id, &bytes[offset..end]));
        offset = end;
    }
    if offset != bytes.len() {
        return Err(MigrationError::TrailingBytes);
    }
    Ok(fields)
}

/// 单个字段的期望形态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// 稳定的字段 ID。
    pub id: u16,
    /// ID 缺失时使用的默认负载；为 `None` 表示必需字段。
    pub default: Option<&'static [u8]>,
}

impl FieldSpec {
    /// 负载中必须存在的字段。
    #[must_use]
    pub const fn required(id: u16) -> Self {
        Self { id, default: None }
    }

    /// 允许缺失的字段；缺失时使用 `default`。
    #[must_use]
    pub const fn optional(id: u16, default: &'static [u8]) -> Self {
        Self {
            id,
            default: Some(default),
        }
    }
}

/// 迁移后的单个字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigratedField<'a> {
    /// 稳定的字段 ID。
    pub id: u16,
    /// 字段负载。
    pub bytes: Cow<'a, [u8]>,
    /// 该负载是否由 [`FieldSpec::default`] 提供。
    pub from_default: bool,
}

/// 按 `schema` 重排、丢弃并填充字段。
///
/// 未知 ID 会被丢弃，保留 schema 顺序，新字段取其默认值。
/// 新版 guest 会对旧版 guest 导出的字节调用此函数。
///
///
/// # Errors
/// 缺少必需字段时返回 [`MigrationError::MissingField`]，
/// 结构性错误同 [`decode`]。
pub fn migrate<'a>(
    bytes: &'a [u8],
    schema: &[FieldSpec],
) -> Result<Vec<MigratedField<'a>>, MigrationError> {
    let incoming = decode(bytes)?;
    let mut out = Vec::with_capacity(schema.len());
    for spec in schema {
        match incoming.iter().find(|field| field.id == spec.id) {
            Some(field) => out.push(MigratedField {
                id: spec.id,
                bytes: Cow::Borrowed(field.bytes),
                from_default: false,
            }),
            None => match spec.default {
                Some(default) => out.push(MigratedField {
                    id: spec.id,
                    bytes: Cow::Borrowed(default),
                    from_default: true,
                }),
                None => return Err(MigrationError::MissingField(spec.id)),
            },
        }
    }
    Ok(out)
}

/// 列表头部长度，即 `u32` 元素计数。
const LIST_HEADER_LEN: usize = 4;
/// 单个元素 `u32` 信封长度字段的字节数。
const ITEM_LENGTH_LEN: usize = 4;

/// 将集合编码为 `u32 count | count × (u32 length | UBMF envelope)`。
///
/// 这种嵌套结构使 `Row` 变更开销极低：每个元素各自拥有带字段标签的信封，
/// 因此向某一行新增字段不会移动其他行的字节偏移，解码器可独立迁移每一项。
///
///
/// # Errors
/// 透传编码器与 [`encode`] 返回的错误。
///
pub fn encode_list<T, F>(items: &[T], mut encode_item: F) -> Result<Vec<u8>, MigrationError>
where
    F: FnMut(&T) -> Result<Vec<u8>, MigrationError>,
{
    let count = u32::try_from(items.len()).map_err(|_| MigrationError::TooManyFields)?;
    let mut out = Vec::with_capacity(LIST_HEADER_LEN + items.len() * 8);
    out.extend_from_slice(&count.to_le_bytes());
    for item in items {
        let envelope = encode_item(item)?;
        let len = u32::try_from(envelope.len()).map_err(|_| MigrationError::FieldTooLarge)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&envelope);
    }
    Ok(out)
}

/// 解码并迁移由 [`encode_list`] 编码的列表。
///
/// 每一项均独立按 `item_schema` 迁移，因此新增字段会取默认值，
/// 移除字段会被丢弃，且不影响相邻元素。
///
///
/// # Errors
/// 当列表被截断、存在尾部多余字节或任意元素迁移失败时，返回结构性 [`MigrationError`]。
///
pub fn migrate_list<T, F>(
    bytes: &[u8],
    item_schema: &[FieldSpec],
    mut decode_item: F,
) -> Result<Vec<T>, MigrationError>
where
    F: FnMut(&[MigratedField<'_>]) -> Result<T, MigrationError>,
{
    if bytes.len() < LIST_HEADER_LEN {
        return Err(MigrationError::Truncated);
    }
    let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut offset = LIST_HEADER_LEN;
    // 损坏的计数不得预先分配数 GB 内存。
    let mut items = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        if bytes.len() - offset < ITEM_LENGTH_LEN {
            return Err(MigrationError::Truncated);
        }
        let len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += ITEM_LENGTH_LEN;
        let end = offset.checked_add(len).ok_or(MigrationError::Truncated)?;
        if end > bytes.len() {
            return Err(MigrationError::Truncated);
        }
        let fields = migrate(&bytes[offset..end], item_schema)?;
        items.push(decode_item(&fields)?);
        offset = end;
    }
    if offset != bytes.len() {
        return Err(MigrationError::TrailingBytes);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip_in_order() {
        let bytes = encode(&[Field::new(2, b"title"), Field::new(1, b"\x05")]).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(
            decoded,
            vec![Field::new(2, b"title"), Field::new(1, b"\x05")]
        );
    }

    #[test]
    fn a_new_field_takes_its_default() {
        let old = encode(&[Field::new(1, b"count")]).unwrap();
        let schema = [
            FieldSpec::required(1),
            FieldSpec::optional(2, b"default title"),
        ];
        let migrated = migrate(&old, &schema).unwrap();
        assert_eq!(migrated.len(), 2);
        assert_eq!(migrated[0].bytes.as_ref(), b"count");
        assert!(!migrated[0].from_default);
        assert_eq!(migrated[1].bytes.as_ref(), b"default title");
        assert!(migrated[1].from_default);
    }

    #[test]
    fn a_removed_field_is_dropped_and_order_follows_the_new_schema() {
        let old = encode(&[
            Field::new(1, b"one"),
            Field::new(9, b"gone"),
            Field::new(2, b"two"),
        ])
        .unwrap();
        let schema = [FieldSpec::required(2), FieldSpec::required(1)];
        let migrated = migrate(&old, &schema).unwrap();
        let ids: Vec<u16> = migrated.iter().map(|field| field.id).collect();
        assert_eq!(ids, vec![2, 1]);
        assert_eq!(migrated[0].bytes.as_ref(), b"two");
    }

    #[test]
    fn a_missing_required_field_is_an_error() {
        let old = encode(&[Field::new(1, b"count")]).unwrap();
        let error = migrate(&old, &[FieldSpec::required(2)]).unwrap_err();
        assert_eq!(error, MigrationError::MissingField(2));
        assert_eq!(error.to_string(), "missing required field 2");
    }

    #[test]
    fn structural_corruption_is_rejected() {
        assert_eq!(decode(&[]), Err(MigrationError::Truncated));
        assert_eq!(
            decode(b"NOPE\x01\x00\x00\x00"),
            Err(MigrationError::BadMagic)
        );

        let mut future = encode(&[]).unwrap();
        future[4] = 99;
        assert_eq!(decode(&future), Err(MigrationError::UnsupportedVersion(99)));

        let mut truncated = encode(&[Field::new(1, b"hello")]).unwrap();
        truncated.pop();
        assert_eq!(decode(&truncated), Err(MigrationError::Truncated));

        let mut trailing = encode(&[]).unwrap();
        trailing.push(0);
        assert_eq!(decode(&trailing), Err(MigrationError::TrailingBytes));
    }

    #[test]
    fn duplicate_field_ids_are_rejected() {
        let bytes = encode(&[Field::new(1, b"a"), Field::new(1, b"b")]).unwrap();
        assert_eq!(decode(&bytes), Err(MigrationError::DuplicateField(1)));
    }

    #[test]
    fn an_empty_schema_accepts_an_empty_envelope() {
        let bytes = encode(&[]).unwrap();
        assert!(decode(&bytes).unwrap().is_empty());
        assert!(migrate(&bytes, &[]).unwrap().is_empty());
    }

    #[test]
    fn a_field_length_cannot_exceed_the_header() {
        let mut bytes = encode(&[]).unwrap();
        // 一条声称拥有 0xFFFF_FFFF 字节负载的记录。
        bytes[6] = 1;
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(MigrationError::Truncated));
    }

    // ---- 嵌套列表 ---------------------------------------------------

    #[derive(Debug, PartialEq)]
    struct Item {
        name: String,
        done: bool,
        priority: u8,
    }

    const ITEM_NAME: u16 = 1;
    const ITEM_DONE: u16 = 2;
    const ITEM_PRIORITY: u16 = 3;

    const ITEM_V1: &[FieldSpec] = &[
        FieldSpec::required(ITEM_NAME),
        FieldSpec::required(ITEM_DONE),
    ];
    const ITEM_V2: &[FieldSpec] = &[
        FieldSpec::required(ITEM_NAME),
        FieldSpec::required(ITEM_DONE),
        FieldSpec::optional(ITEM_PRIORITY, &[0]),
    ];

    fn encode_item(item: &Item, schema: &[FieldSpec]) -> Result<Vec<u8>, MigrationError> {
        let name = postcard::to_allocvec(&item.name).unwrap();
        let done = postcard::to_allocvec(&item.done).unwrap();
        let priority = postcard::to_allocvec(&item.priority).unwrap();
        let mut fields = vec![Field::new(ITEM_NAME, &name), Field::new(ITEM_DONE, &done)];
        if schema.iter().any(|spec| spec.id == ITEM_PRIORITY) {
            fields.push(Field::new(ITEM_PRIORITY, &priority));
        }
        encode(&fields)
    }

    fn decode_item(fields: &[MigratedField<'_>]) -> Result<Item, MigrationError> {
        let read = |id: u16| fields.iter().find(|field| field.id == id).unwrap();
        Ok(Item {
            name: postcard::from_bytes(&read(ITEM_NAME).bytes).unwrap(),
            done: postcard::from_bytes(&read(ITEM_DONE).bytes).unwrap(),
            priority: postcard::from_bytes(&read(ITEM_PRIORITY).bytes).unwrap(),
        })
    }

    fn sample_items(count: usize, priority: u8) -> Vec<Item> {
        (0..count)
            .map(|index| Item {
                name: format!("row {index}"),
                done: index % 3 == 0,
                priority,
            })
            .collect()
    }

    #[test]
    fn a_list_round_trips_each_item_as_its_own_envelope() {
        let items = sample_items(100, 0);
        let bytes = encode_list(&items, |item| encode_item(item, ITEM_V2)).unwrap();
        let decoded = migrate_list(&bytes, ITEM_V2, decode_item).unwrap();
        assert_eq!(decoded, items);
    }

    #[test]
    fn every_item_of_a_v1_list_gains_the_v2_default_independently() {
        let items = sample_items(100, 0);
        let bytes = encode_list(&items, |item| encode_item(item, ITEM_V1)).unwrap();
        let decoded = migrate_list(&bytes, ITEM_V2, decode_item).unwrap();
        assert_eq!(decoded.len(), 100);
        for (old, new) in items.iter().zip(&decoded) {
            assert_eq!(new.name, old.name);
            assert_eq!(new.done, old.done);
            assert_eq!(new.priority, 0, "the new field takes its default");
        }
    }

    #[test]
    fn a_truncated_list_is_rejected_without_allocating_the_claimed_count() {
        let items = sample_items(3, 1);
        let mut bytes = encode_list(&items, |item| encode_item(item, ITEM_V2)).unwrap();
        bytes.truncate(bytes.len() - 1);
        assert_eq!(
            migrate_list::<Item, _>(&bytes, ITEM_V2, decode_item),
            Err(MigrationError::Truncated)
        );

        // 读取字节前绝不可轻信声明的超大计数。
        let mut lie = vec![0xFF, 0xFF, 0xFF, 0xFF];
        lie.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            migrate_list::<Item, _>(&lie, ITEM_V2, decode_item),
            Err(MigrationError::Truncated)
        );
    }

    #[test]
    fn trailing_bytes_after_a_list_are_rejected() {
        let items = sample_items(1, 0);
        let mut bytes = encode_list(&items, |item| encode_item(item, ITEM_V2)).unwrap();
        bytes.push(0);
        assert_eq!(
            migrate_list::<Item, _>(&bytes, ITEM_V2, decode_item),
            Err(MigrationError::TrailingBytes)
        );
    }
}
