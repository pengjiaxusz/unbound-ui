//! Versioned, field-tagged state migration.
//!
//! The schema hash in [`crate::schema`] answers "same layout?". When the answer
//! is no, the guest still has to carry the user's data across the reload. A
//! monolithic `postcard` blob cannot do that: adding one field at the end
//! changes every byte offset, and a missing field is indistinguishable from a
//! corrupt one.
//!
//! The envelope here is the smallest format that survives both:
//!
//! ```text
//! "UBMF" | u16 version | u16 count | count × { u16 field_id, u32 len, len bytes }
//! ```
//!
//! * Fields are encoded one by one by the guest (each with `postcard`), so a
//!   field can be added or removed independently.
//! * [`migrate`] reorders, drops unknown fields and fills new ones from a
//!   declared default.
//! * A missing **required** field is an error, not a silently zeroed value —
//!   the host can then fall back to a fresh state and say so.
//!
//! The format is intentionally boring: fixed-width little-endian headers and
//! opaque payloads, so it can be parsed by a host-side tool without linking
//! the guest.

use std::borrow::Cow;

/// Envelope magic, `UBMF` ("Unbound migration format").
pub const MAGIC: [u8; 4] = *b"UBMF";
/// The current envelope version.
pub const ENVELOPE_VERSION: u16 = 1;
const HEADER_LEN: usize = 8;
const RECORD_HEADER_LEN: usize = 6;

/// One encoded field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    /// The stable field id.
    pub id: u16,
    /// The field payload, opaque to this crate.
    pub bytes: &'a [u8],
}

impl<'a> Field<'a> {
    #[must_use]
    pub const fn new(id: u16, bytes: &'a [u8]) -> Self {
        Self { id, bytes }
    }
}

/// Why decoding or migration failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationError {
    /// The envelope is shorter than its header claims.
    Truncated,
    /// The magic does not match [`MAGIC`].
    BadMagic,
    /// The envelope version is newer than this build understands.
    UnsupportedVersion(u16),
    /// The same field id appeared twice.
    DuplicateField(u16),
    /// Bytes remain after the last record.
    TrailingBytes,
    /// The record count does not fit the `u16` header.
    TooManyFields,
    /// A payload does not fit the `u32` length field.
    FieldTooLarge,
    /// A required field is absent from the payload.
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

/// Encode fields into an envelope.
///
/// # Errors
/// Returns [`MigrationError::TooManyFields`] or [`MigrationError::FieldTooLarge`]
/// when the payload cannot be represented by the header.
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

/// Decode an envelope without interpreting the payloads.
///
/// # Errors
/// Returns a [`MigrationError`] when the payload is corrupt, uses an unknown
/// version, repeats a field id, or has trailing bytes.
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

/// The expected shape of one field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// The stable field id.
    pub id: u16,
    /// A payload to use when the id is absent. `None` makes it required.
    pub default: Option<&'static [u8]>,
}

impl FieldSpec {
    /// A field that must exist in the payload.
    #[must_use]
    pub const fn required(id: u16) -> Self {
        Self { id, default: None }
    }

    /// A field that may be absent; `default` is used when it is.
    #[must_use]
    pub const fn optional(id: u16, default: &'static [u8]) -> Self {
        Self {
            id,
            default: Some(default),
        }
    }
}

/// One field after migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigratedField<'a> {
    /// The stable field id.
    pub id: u16,
    /// The field payload.
    pub bytes: Cow<'a, [u8]>,
    /// Whether [`FieldSpec::default`] supplied the payload.
    pub from_default: bool,
}

/// Reorder, drop and fill fields according to `schema`.
///
/// Unknown ids are dropped, schema order is preserved, and new fields take
/// their defaults. This is the function a new guest calls on the bytes the old
/// guest exported.
///
/// # Errors
/// Returns [`MigrationError::MissingField`] when a required field is absent,
/// and the same structural errors as [`decode`].
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
        // One record claiming a 0xFFFF_FFFF byte payload.
        bytes[6] = 1;
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(MigrationError::Truncated));
    }
}
