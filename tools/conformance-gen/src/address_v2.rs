//! Identity and version 2 address conformance vectors, written under
//! `address-v2/` (`client-compute.md` sections 3, 10.3, and 10.5).
//!
//! `cases.json` holds four lists:
//!
//! - `frames`: frames slot values, each with its record hashes in config
//!   order and the expected value.
//! - `input_spans`: input span index cases, all decimal strings, with the
//!   expected index or code.
//! - `identities`: layer identities. Each gives the inputs as
//!   `layer_identity` takes them (hashes as lowercase hex, the manifest
//!   `null` for none, integers as decimal strings), the slots as
//!   `{kind, value}` with `kind` `"layer"` or `"frames"`, the preimage as hex,
//!   and the identity. A layer slot names the identity it holds in `of`, a
//!   frames slot the `frames` case in `of`. An epoch layer's case carries its
//!   epoch index as hex in `epoch_index`: its manifest is the epoch rule's
//!   answer for the case's span.
//! - `addresses`: version 2 addresses, the same fields plus the chunk key,
//!   with the preimage and address, or the expected code.
//!
//! Module, manifest, and record hashes are SHA-256 digests of fixed short
//! strings.

use gx_core::address::{
    address_preimage_v2, frames_slot_value, identity_preimage, input_span_index, layer_identity,
    section_address_v2, SlotKind, SlotValue,
};
use gx_core::epochs::{self, EpochIndex};
use gx_core::key::ChunkKey;
use gx_core::sha256::{sha256, to_hex};
use serde_json::{json, Value};

use crate::{pretty, File};

/// An origin in UTC milliseconds used by the cases with `usesOrigin` on.
const ORIGIN: i64 = 1_763_130_000_000;

/// One layer at one span, as an identity or an address takes it.
struct Layer {
    name: &'static str,
    module: [u8; 32],
    manifest: Option<[u8; 32]>,
    origin: Option<i64>,
    span_seconds: u64,
    span_index: u64,
    /// Each slot with the name of the case its value comes from.
    slots: Vec<(SlotValue, &'static str)>,
}

impl Layer {
    fn values(&self) -> Vec<SlotValue> {
        self.slots.iter().map(|s| s.0).collect()
    }

    fn origin_utc_ms(&self) -> i64 {
        self.origin.unwrap_or(0)
    }

    fn identity(&self) -> [u8; 32] {
        layer_identity(
            &self.module,
            self.manifest.as_ref(),
            self.origin_utc_ms(),
            self.span_seconds,
            self.span_index,
            &self.values(),
        )
        .expect("fixed slots")
    }

    fn json(&self) -> Value {
        let slots: Vec<Value> = self
            .slots
            .iter()
            .map(|(s, of)| {
                json!({
                    "kind": match s.kind { SlotKind::Layer => "layer", SlotKind::Frames => "frames" },
                    "value": to_hex(&s.value),
                    "of": of,
                })
            })
            .collect();
        json!({
            "name": self.name,
            "module": to_hex(&self.module),
            "manifest": self.manifest.map(|m| to_hex(&m)),
            "uses_origin": self.origin.is_some(),
            "origin_utc_ms": self.origin_utc_ms().to_string(),
            "span_seconds": self.span_seconds.to_string(),
            "span_index": self.span_index.to_string(),
            "slots": slots,
        })
    }

    fn identity_json(&self) -> Value {
        let mut v = self.json();
        let pre = identity_preimage(
            &self.module,
            self.manifest.as_ref(),
            self.origin_utc_ms(),
            self.span_seconds,
            self.span_index,
            &self.values(),
        )
        .expect("fixed slots");
        v["preimage"] = json!(to_hex(&pre));
        v["identity"] = json!(to_hex(&self.identity()));
        v
    }

    fn address_json(&self, key: &str) -> Value {
        let mut v = self.json();
        v["key"] = json!(key);
        let k: ChunkKey = key.parse().expect("fixed keys are valid");
        let args = (
            &self.module,
            self.manifest.as_ref(),
            self.origin_utc_ms(),
            self.span_seconds,
            self.span_index,
        );
        match address_preimage_v2(args.0, args.1, args.2, args.3, args.4, &self.values(), &k) {
            Ok(pre) => {
                let a =
                    section_address_v2(args.0, args.1, args.2, args.3, args.4, &self.values(), &k)
                        .expect("same inputs");
                v["preimage"] = json!(to_hex(&pre));
                v["address"] = json!(to_hex(&a));
            }
            Err(e) => v["code"] = json!(e.code),
        }
        v
    }
}

fn layer(v: [u8; 32], of: &'static str) -> (SlotValue, &'static str) {
    (
        SlotValue {
            kind: SlotKind::Layer,
            value: v,
        },
        of,
    )
}

fn frames(v: [u8; 32], of: &'static str) -> (SlotValue, &'static str) {
    (
        SlotValue {
            kind: SlotKind::Frames,
            value: v,
        },
        of,
    )
}

fn span_cases() -> Value {
    let cases: Vec<Value> = [
        (0u64, 0u64, 0u64),
        (30, 3600, 0),
        (30, 3600, 3600),
        (30, 3600, 86_400),
        (23, 3600, 86_400),
        (24, 3600, 86_400),
        (1, 86_400, 3600),
        (u64::MAX, u64::MAX, u64::MAX),
        (u64::MAX, 60, 1),
    ]
    .iter()
    .map(|&(i, r, s)| {
        let mut v = json!({
            "reader_span_index": i.to_string(),
            "reader_span_seconds": r.to_string(),
            "input_span_seconds": s.to_string(),
        });
        match input_span_index(i, r, s) {
            Ok(x) => v["input_span_index"] = json!(x.to_string()),
            Err(e) => v["code"] = json!(e.code),
        }
        v
    })
    .collect();
    json!(cases)
}

fn static_like(t: &Layer) -> Layer {
    Layer {
        name: t.name,
        module: t.module,
        manifest: t.manifest,
        origin: t.origin,
        span_seconds: t.span_seconds,
        span_index: t.span_index,
        slots: vec![],
    }
}

/// Every identity and version 2 address vector file.
pub fn files() -> Vec<File> {
    // The frames slot: records of two frames, in config order.
    let records = [sha256(b"frame record 3"), sha256(b"frame record 5")];
    let fv = frames_slot_value(&records);
    let frames_json = json!([
        {
            "name": "frames-3-5",
            "records": records.iter().map(|r| to_hex(r)).collect::<Vec<_>>(),
            "value": to_hex(&fv),
        },
        {
            "name": "frames-none",
            "records": [],
            "value": to_hex(&frames_slot_value(&[])),
        },
    ]);

    // A static layer with a manifest, no inputs.
    let static_layer = Layer {
        name: "static-no-inputs",
        module: sha256(b"module static"),
        manifest: Some(sha256(b"source manifest static")),
        origin: None,
        span_seconds: 0,
        span_index: 0,
        slots: vec![],
    };
    // An epoch layer: hourly spans, daily epochs.
    let index = EpochIndex::new(
        86_400,
        vec![
            epochs::Entry {
                epoch: 0,
                manifest: sha256(b"source manifest day 0"),
            },
            epochs::Entry {
                epoch: 1,
                manifest: sha256(b"source manifest day 1"),
            },
        ],
    )
    .expect("fixed index");
    let index_bytes = epochs::encode(&index);
    let epoch_layer = |name, span_index| Layer {
        name,
        module: sha256(b"module epoch"),
        manifest: epochs::manifest_for_span(&index, 3600, span_index).expect("rule"),
        origin: None,
        span_seconds: 3600,
        span_index,
        slots: vec![],
    };
    let epoch_30 = epoch_layer("epoch-layer-span-30", 30);
    let epoch_24 = epoch_layer("epoch-layer-span-24", 24);
    // A reader with both slot kinds, on the origin, at spans 30 and 24.
    let both = |name, span_index, e: &Layer, of| Layer {
        name,
        module: sha256(b"module reader"),
        manifest: None,
        origin: Some(ORIGIN),
        span_seconds: 3600,
        span_index,
        slots: vec![layer(e.identity(), of), frames(fv, "frames-3-5")],
    };
    let both_30 = both("both-span-30", 30, &epoch_30, "epoch-layer-span-30");
    let both_24 = both("both-span-24", 24, &epoch_24, "epoch-layer-span-24");
    // A daily layer reading the hourly reader at span 1 reads it at 24.
    let nested = Layer {
        name: "nested-day-1",
        module: sha256(b"module nested"),
        manifest: None,
        origin: Some(ORIGIN),
        span_seconds: 86_400,
        span_index: 1,
        slots: vec![layer(both_24.identity(), "both-span-24")],
    };
    let one_layer = Layer {
        name: "one-layer-slot",
        module: sha256(b"module one layer"),
        manifest: Some(sha256(b"source manifest one layer")),
        origin: None,
        span_seconds: 0,
        span_index: 0,
        slots: vec![layer(static_layer.identity(), "static-no-inputs")],
    };
    let one_frames = Layer {
        name: "one-frames-slot",
        module: sha256(b"module one frames"),
        manifest: None,
        origin: None,
        span_seconds: 0,
        span_index: 0,
        slots: vec![frames(fv, "frames-3-5")],
    };
    let no_inputs = Layer {
        name: "no-inputs-refused",
        ..static_like(&static_layer)
    };

    let mut epoch_json = epoch_30.identity_json();
    epoch_json["epoch_index"] = json!(to_hex(&index_bytes));
    let mut epoch_24_json = epoch_24.identity_json();
    epoch_24_json["epoch_index"] = json!(to_hex(&index_bytes));
    let identities = vec![
        static_layer.identity_json(),
        epoch_json,
        epoch_24_json,
        one_layer.identity_json(),
        one_frames.identity_json(),
        both_30.identity_json(),
        both_24.identity_json(),
        nested.identity_json(),
    ];
    let addresses = vec![
        no_inputs.address_json("1-0-0-0-0"),
        one_layer.address_json("1-0-0-0-0"),
        one_frames.address_json("registry"),
        both_30.address_json("4-12-1024-2048-3071"),
        both_30.address_json("frame-3"),
        nested.address_json("2-0-0-0-0"),
    ];
    vec![(
        "address-v2/cases.json".into(),
        pretty(&json!({
            "frames": frames_json,
            "input_spans": span_cases(),
            "identities": identities,
            "addresses": addresses,
        })),
    )]
}
