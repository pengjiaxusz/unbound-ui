//! 嵌套集合必须在 schema 变更后保持兼容。
//!
//! V1 使用位置敏感的 `postcard` 存储 `Vec<RowV1>`，因此向行中新增字段会
//! 偏移后续所有字节，导致整个列表解码失败。如今每行各自作为独立的自描述
//! UBMF 信封传输：新 guest 独立迁移每一行，保留已知字段并用声明的默认值
//! 填充新字段。因此 V1 负载可完整加载而不丢失任何一行。
//!

use serde::{Deserialize, Serialize};
use unbound_abi::migration::{
    encode, encode_list, migrate, migrate_list, Field, FieldSpec, MigratedField, MigrationError,
};

const ROW_LABEL: u16 = 1;
const ROW_DONE: u16 = 2;
const ROW_PRIORITY: u16 = 3;
const ROW_TAGS: u16 = 4;
/// V2 移除的字段；新 guest 必须丢弃它，而非解析崩溃。
const ROW_LEGACY_NOTE: u16 = 9;
const EMPTY_COLLECTION: &[u8] = &[0];

const ROW_V1_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(ROW_LABEL),
    FieldSpec::required(ROW_DONE),
];
const ROW_V2_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(ROW_LABEL),
    FieldSpec::required(ROW_DONE),
    FieldSpec::optional(ROW_PRIORITY, EMPTY_COLLECTION),
    FieldSpec::optional(ROW_TAGS, EMPTY_COLLECTION),
];

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct RowV1 {
    label: String,
    done: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct RowV2 {
    label: String,
    done: bool,
    priority: u8,
    tags: Vec<String>,
}

/// 通过 V1 guest 导出：每行一个信封，不含 priority 或 tags。
fn export_v1(rows: &[RowV1]) -> Vec<u8> {
    encode_list(rows, |row| {
        let label = postcard::to_allocvec(&row.label).unwrap();
        let done = postcard::to_allocvec(&row.done).unwrap();
        // 已移除的字段随旧数据携带；迁移必须将其丢弃。
        let legacy = postcard::to_allocvec("old note").unwrap();
        encode(&[
            Field::new(ROW_LABEL, &label),
            Field::new(ROW_DONE, &done),
            Field::new(ROW_LEGACY_NOTE, &legacy),
        ])
    })
    .unwrap()
}

fn read<T: serde::de::DeserializeOwned>(fields: &[MigratedField<'_>], id: u16) -> T {
    let field = fields
        .iter()
        .find(|field| field.id == id)
        .unwrap_or_else(|| panic!("field {id} is absent"));
    postcard::from_bytes(&field.bytes).unwrap()
}

/// 通过 V2 guest 导入。
fn import_v2(bytes: &[u8]) -> Vec<RowV2> {
    migrate_list(bytes, ROW_V2_FIELDS, |fields| {
        Ok(RowV2 {
            label: read(fields, ROW_LABEL),
            done: read(fields, ROW_DONE),
            priority: read(fields, ROW_PRIORITY),
            tags: read(fields, ROW_TAGS),
        })
    })
    .unwrap()
}

fn hundred_rows() -> Vec<RowV1> {
    (0..100)
        .map(|index| RowV1 {
            label: format!("task {index}"),
            done: index % 4 == 0,
        })
        .collect()
}

#[test]
fn a_hundred_v1_rows_keep_their_data_and_gain_v2_defaults() {
    let old = hundred_rows();
    let new = import_v2(&export_v1(&old));
    assert_eq!(new.len(), 100, "not one row may be lost");
    for (before, after) in old.iter().zip(&new) {
        assert_eq!(after.label, before.label);
        assert_eq!(after.done, before.done);
        assert_eq!(after.priority, 0, "the new field takes its default");
        assert!(
            after.tags.is_empty(),
            "the new collection defaults to empty"
        );
    }
}

#[test]
fn a_v2_list_round_trips_through_the_same_envelopes() {
    let rows: Vec<RowV2> = (0..50)
        .map(|index| RowV2 {
            label: format!("task {index}"),
            done: index % 2 == 0,
            priority: (index % 5) as u8,
            tags: vec!["ui".into(), "guest".into()],
        })
        .collect();
    let bytes = encode_list(&rows, |row| {
        let label = postcard::to_allocvec(&row.label).unwrap();
        let done = postcard::to_allocvec(&row.done).unwrap();
        let priority = postcard::to_allocvec(&row.priority).unwrap();
        let tags = postcard::to_allocvec(&row.tags).unwrap();
        encode(&[
            Field::new(ROW_LABEL, &label),
            Field::new(ROW_DONE, &done),
            Field::new(ROW_PRIORITY, &priority),
            Field::new(ROW_TAGS, &tags),
        ])
    })
    .unwrap();
    assert_eq!(import_v2(&bytes), rows);
}

#[test]
fn every_row_is_migrated_on_its_own() {
    // 单行损坏必须使列表解码失败，而非把下一行的字节当作当前行来解码。
    //
    let mut bytes = export_v1(&hundred_rows()[..3]);
    // 第二行起始于：header(4) + len(4) + 第 0 行信封。
    let first = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let second_len_at = 4 + 4 + first;
    let second = u32::from_le_bytes([
        bytes[second_len_at],
        bytes[second_len_at + 1],
        bytes[second_len_at + 2],
        bytes[second_len_at + 3],
    ]) as usize;
    // 丢弃第 1 行信封的最后一个字节：其声明长度此时将越界。
    let cut = second_len_at + 4 + second - 1;
    bytes.truncate(cut);
    let error = migrate_list(bytes.as_slice(), ROW_V2_FIELDS, |fields| {
        Ok(RowV2 {
            label: read(fields, ROW_LABEL),
            done: read(fields, ROW_DONE),
            priority: read(fields, ROW_PRIORITY),
            tags: read(fields, ROW_TAGS),
        })
    })
    .unwrap_err();
    assert!(matches!(
        error,
        MigrationError::Truncated | MigrationError::MissingField(_)
    ));
}

#[test]
fn a_row_missing_a_required_field_is_rejected_visibly() {
    let label = postcard::to_allocvec("no done flag").unwrap();
    let bytes = encode_list(&[()], |()| encode(&[Field::new(ROW_LABEL, &label)])).unwrap();
    let error = migrate_list::<RowV2, _>(&bytes, ROW_V2_FIELDS, |fields| {
        Ok(RowV2 {
            label: read(fields, ROW_LABEL),
            done: read(fields, ROW_DONE),
            priority: read(fields, ROW_PRIORITY),
            tags: read(fields, ROW_TAGS),
        })
    })
    .unwrap_err();
    assert_eq!(error, MigrationError::MissingField(ROW_DONE));
}

#[test]
fn a_removed_row_field_is_dropped_by_the_v2_schema() {
    let old = export_v1(&hundred_rows()[..1]);
    let mut offset = 8; // 列表头部 + 首行长度
    let len = u32::from_le_bytes([old[4], old[5], old[6], old[7]]) as usize;
    let row = &old[offset..offset + len];
    offset += len;
    assert_eq!(offset, old.len());
    let fields = migrate(row, ROW_V1_FIELDS).unwrap();
    assert_eq!(fields.len(), 2, "legacy note dropped, required kept");
    assert!(fields.iter().all(|field| field.id != ROW_LEGACY_NOTE));
}
