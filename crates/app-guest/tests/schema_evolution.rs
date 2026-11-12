//! A state that gained and lost a field must still load.
//!
//! The envelope is what makes a hot reload survive a schema change: the new
//! guest reads the old guest's bytes, drops fields it no longer knows, takes a
//! default for a field that did not exist yet, and refuses a payload missing a
//! required field instead of silently decoding garbage.

use serde::{Deserialize, Serialize};
use unbound_abi::migration::{encode, migrate, Field, FieldSpec};

const FIELD_COUNT: u16 = 1;
const FIELD_TITLE: u16 = 2;
const FIELD_ROWS: u16 = 3;
const FIELD_LEGACY_SCROLL: u16 = 9;
const EMPTY_COLLECTION: &[u8] = &[0];

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct V1 {
    count: i64,
    rows: Vec<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct V2 {
    count: i64,
    title: String,
    rows: Vec<String>,
}

fn export_v1(state: &V1) -> Vec<u8> {
    encode(&[
        Field::new(FIELD_COUNT, &postcard::to_allocvec(&state.count).unwrap()),
        Field::new(FIELD_ROWS, &postcard::to_allocvec(&state.rows).unwrap()),
        // A field the new version removed entirely.
        Field::new(FIELD_LEGACY_SCROLL, b"\x2a"),
    ])
    .unwrap()
}

fn import_v2(bytes: &[u8]) -> V2 {
    let schema = [
        FieldSpec::required(FIELD_COUNT),
        FieldSpec::optional(FIELD_TITLE, EMPTY_COLLECTION),
        FieldSpec::required(FIELD_ROWS),
    ];
    let fields = migrate(bytes, &schema).unwrap();
    let read = |id: u16| fields.iter().find(|field| field.id == id).unwrap();
    V2 {
        count: postcard::from_bytes(&read(FIELD_COUNT).bytes).unwrap(),
        title: postcard::from_bytes(&read(FIELD_TITLE).bytes).unwrap(),
        rows: postcard::from_bytes(&read(FIELD_ROWS).bytes).unwrap(),
    }
}

#[test]
fn a_field_added_by_v2_defaults_when_loading_v1_bytes() {
    let old = V1 {
        count: 3,
        rows: vec!["one".into(), "two".into()],
    };
    let bytes = export_v1(&old);
    let new = import_v2(&bytes);
    assert_eq!(new.count, 3);
    assert_eq!(new.rows, old.rows);
    assert_eq!(new.title, "", "the new field takes its default");
}

#[test]
fn a_field_removed_in_v2_is_dropped_but_the_rest_survives() {
    let old = V1 {
        count: 1,
        rows: vec!["row".into()],
    };
    let bytes = export_v1(&old);
    let fields = migrate(
        &bytes,
        &[
            FieldSpec::required(FIELD_COUNT),
            FieldSpec::required(FIELD_ROWS),
        ],
    )
    .unwrap();
    assert_eq!(fields.len(), 2);
    assert!(fields.iter().all(|field| field.id != FIELD_LEGACY_SCROLL));
}

#[test]
fn a_v2_payload_round_trips_through_the_same_envelope() {
    let state = V2 {
        count: 2,
        title: "Notes".into(),
        rows: vec!["a".into()],
    };
    let bytes = encode(&[
        Field::new(FIELD_COUNT, &postcard::to_allocvec(&state.count).unwrap()),
        Field::new(FIELD_TITLE, &postcard::to_allocvec(&state.title).unwrap()),
        Field::new(FIELD_ROWS, &postcard::to_allocvec(&state.rows).unwrap()),
    ])
    .unwrap();
    assert_eq!(import_v2(&bytes), state);
}

#[test]
fn a_payload_missing_a_required_field_is_rejected_visibly() {
    let bytes = encode(&[Field::new(
        FIELD_ROWS,
        &postcard::to_allocvec(&Vec::<String>::new()).unwrap(),
    )])
    .unwrap();
    let error = migrate(
        &bytes,
        &[
            FieldSpec::required(FIELD_COUNT),
            FieldSpec::required(FIELD_ROWS),
        ],
    )
    .unwrap_err();
    assert_eq!(
        error,
        unbound_abi::migration::MigrationError::MissingField(FIELD_COUNT)
    );
}
