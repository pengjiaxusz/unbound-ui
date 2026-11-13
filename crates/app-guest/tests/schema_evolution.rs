//! A nested collection must survive a schema change.
//!
//! V1 stored `Vec<RowV1>` with positional `postcard`, so adding a field to a
//! row shifted every following byte and the whole list failed to decode. Rows
//! now travel as one self-describing UBMF envelope each: the new guest migrates
//! every row on its own, keeps the fields it knows and fills the new ones from
//! their declared default. A V1 payload therefore loads without losing a row.

use serde::{Deserialize, Serialize};
use unbound_abi::migration::{
    encode, encode_list, migrate, migrate_list, Field, FieldSpec, MigratedField, MigrationError,
};

const ROW_LABEL: u16 = 1;
const ROW_DONE: u16 = 2;
const ROW_PRIORITY: u16 = 3;
const ROW_TAGS: u16 = 4;
/// The field V2 removed; a new guest must drop it, not choke on it.
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

/// Export through the V1 guest: one envelope per row, no priority or tags.
fn export_v1(rows: &[RowV1]) -> Vec<u8> {
    encode_list(rows, |row| {
        let label = postcard::to_allocvec(&row.label).unwrap();
        let done = postcard::to_allocvec(&row.done).unwrap();
        // A removed field rides along; migration must drop it.
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

/// Import through the V2 guest.
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
    // A single corrupt row must fail the list instead of decoding the bytes of
    // the next row as if they belonged to this one.
    let mut bytes = export_v1(&hundred_rows()[..3]);
    // The second row starts after: header(4) + len(4) + envelope of row 0.
    let first = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let second_len_at = 4 + 4 + first;
    let second = u32::from_le_bytes([
        bytes[second_len_at],
        bytes[second_len_at + 1],
        bytes[second_len_at + 2],
        bytes[second_len_at + 3],
    ]) as usize;
    // Drop the last byte of row 1's envelope: its length now overruns.
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
    let mut offset = 8; // list header + first row length
    let len = u32::from_le_bytes([old[4], old[5], old[6], old[7]]) as usize;
    let row = &old[offset..offset + len];
    offset += len;
    assert_eq!(offset, old.len());
    let fields = migrate(row, ROW_V1_FIELDS).unwrap();
    assert_eq!(fields.len(), 2, "legacy note dropped, required kept");
    assert!(fields.iter().all(|field| field.id != ROW_LEGACY_NOTE));
}
