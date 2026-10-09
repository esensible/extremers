//! The shared golden vectors (../vectors/*.json, SPEC §8), consumed directly.
//! Run with `--nocapture` to see the counts.

mod json;

use bitwire::{Descriptor, Error};
use json::{F64Map, expected, hex, shadow, unhex};
use serde_json::Value as Json;

fn read(name: &str) -> Json {
    let path = format!("{}/../vectors/{}", env!("CARGO_MANIFEST_DIR"), name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).expect("vector JSON")
}

fn f64map(case: &Json, key: &str) -> F64Map {
    case.get(key).and_then(Json::as_object).cloned().unwrap_or_default()
}

fn s<'a>(j: &'a Json, k: &str) -> &'a str {
    j[k].as_str().unwrap_or_else(|| panic!("{k}"))
}

#[test]
fn codec_vectors() {
    let v = read("codec.json");
    assert_eq!(v["format"], 1);
    let mut failures: Vec<String> = Vec::new();
    let (mut schemas, mut round_trips, mut decode_only, mut dec_ref, mut enc_ref) = (0, 0, 0, 0, 0);
    for entry in v["schemas"].as_array().unwrap() {
        schemas += 1;
        let name = s(entry, "name");
        let dbytes = unhex(s(entry, "descriptor"));
        let desc = match Descriptor::load(&dbytes) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{name}: descriptor refused: {e}"));
                continue;
            }
        };
        if hex(&desc.hash()) != s(entry, "hash") {
            failures.push(format!("{name}: hash {} != {}", hex(&desc.hash()), s(entry, "hash")));
        }
        let sh = shadow(&desc);

        for case in entry["cases"].as_array().unwrap() {
            let cname = format!("{name} / {}", s(case, "name"));
            let wire = unhex(s(case, "wire"));
            let value = &case["value"];
            let vmap = f64map(case, "value_f64");
            let is_decode_only = case.get("decode_only").and_then(Json::as_bool).unwrap_or(false);
            // `js: "precision"` cases are normal cases for a 64-bit host.
            if !is_decode_only {
                match json::encode(&desc, &sh, value, &vmap) {
                    Ok(b) if b == wire => {}
                    Ok(b) => failures.push(format!("{cname}: encode gave {} want {}", hex(&b), hex(&wire))),
                    Err(e) => failures.push(format!("{cname}: encode refused: {e}")),
                }
            }
            let (want_json, want_map) = match case.get("decoded") {
                Some(d) => (d, f64map(case, "decoded_f64")),
                None => (value, vmap.clone()),
            };
            let want = expected(&sh, want_json, &want_map);
            match json::decode(&desc, &sh, &wire) {
                Ok(got) if got == want => {}
                Ok(got) => failures.push(format!("{cname}: decode gave {got:?} want {want:?}")),
                Err(e) => failures.push(format!("{cname}: decode refused: {e}")),
            }
            if is_decode_only {
                decode_only += 1;
            } else {
                round_trips += 1;
            }
        }

        for r in entry["decode_refusals"].as_array().unwrap() {
            dec_ref += 1;
            let cname = format!("{name} / decode refusal {}", s(r, "name"));
            match json::decode(&desc, &sh, &unhex(s(r, "wire"))) {
                Ok(v) => failures.push(format!("{cname}: decoded {v:?}")),
                Err(e) if e.code() == s(r, "error") => {}
                Err(e) => failures.push(format!("{cname}: refused {e}, want {}", s(r, "error"))),
            }
        }

        for r in entry["encode_refusals"].as_array().unwrap() {
            enc_ref += 1;
            let cname = format!("{name} / encode refusal {}", s(r, "name"));
            match json::encode(&desc, &sh, &r["value"], &F64Map::new()) {
                Ok(b) => failures.push(format!("{cname}: encoded {}", hex(&b))),
                Err(e) if e.code() == s(r, "error") => {}
                Err(e) => failures.push(format!("{cname}: refused {e}, want {}", s(r, "error"))),
            }
        }
    }
    println!(
        "codec.json: {schemas} schemas, {round_trips} round trips, {decode_only} decode-only, \
         {dec_ref} decode refusals, {enc_ref} encode refusals, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn descriptor_refusals() {
    let v = read("descriptors.json");
    assert_eq!(v["format"], 1);
    let mut failures = Vec::new();
    let mut n = 0;
    for r in v["refusals"].as_array().unwrap() {
        n += 1;
        let bytes = unhex(s(r, "descriptor"));
        match Descriptor::load(&bytes) {
            Ok(_) => failures.push(format!("{}: loaded", s(r, "name"))),
            Err(e) if e.code() == s(r, "error") => {}
            Err(e) => failures.push(format!("{}: refused {e}, want {}", s(r, "name"), s(r, "error"))),
        }
    }
    println!("descriptors.json: {n} refusals, {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every proper prefix of every valid descriptor is refused, never a panic.
#[test]
fn descriptor_prefixes_refused() {
    let v = read("codec.json");
    let mut n = 0;
    for entry in v["schemas"].as_array().unwrap() {
        let d = unhex(s(entry, "descriptor"));
        for len in 0..d.len() {
            n += 1;
            assert_eq!(Descriptor::load(&d[..len]).err(), Some(Error::Descriptor), "{} cut at {len}", s(entry, "name"));
        }
    }
    println!("descriptor prefixes: {n} refused");
}

/// Every proper prefix of every message is refused as truncated (the walk
/// reads the same bytes until they run out), and never panics; an output
/// buffer one byte short is refused as `Full`.
#[test]
fn message_prefixes_and_full_buffer() {
    let v = read("codec.json");
    let (mut prefixes, mut full) = (0, 0);
    for entry in v["schemas"].as_array().unwrap() {
        let dbytes = unhex(s(entry, "descriptor"));
        let desc = Descriptor::load(&dbytes).unwrap();
        let sh = shadow(&desc);
        for case in entry["cases"].as_array().unwrap() {
            let wire = unhex(s(case, "wire"));
            for len in 0..wire.len() {
                prefixes += 1;
                let e = json::decode(&desc, &sh, &wire[..len]).expect_err("prefix decoded");
                assert_eq!(e, Error::Truncated, "{}: prefix {len}", s(case, "name"));
            }
            if case.get("decode_only").is_none() && !wire.is_empty() {
                full += 1;
                let mut src = json::Scripted::new(json::script(&sh, &case["value"], &f64map(case, "value_f64")));
                let mut out = vec![0u8; wire.len() - 1];
                assert_eq!(bitwire::encode(&desc, &mut src, &mut out), Err(Error::Full), "{}", s(case, "name"));
            }
        }
    }
    println!("message prefixes: {prefixes} refused; {full} one-byte-short buffers refused as full");
}

/// Mutated descriptors and messages never panic: load either refuses a
/// descriptor or the trusted walk over it is sound (walks the names and
/// decodes mutated messages without panicking). A deterministic xorshift,
/// no dependencies.
#[test]
fn mutations_never_panic() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }
    fn mutate(r: &mut Rng, b: &[u8]) -> Vec<u8> {
        let mut m = b.to_vec();
        for _ in 0..1 + r.below(3) {
            match r.below(4) {
                0 if !m.is_empty() => {
                    let i = r.below(m.len());
                    m[i] ^= 1 << r.below(8);
                }
                1 if !m.is_empty() => {
                    let i = r.below(m.len());
                    m[i] = r.next() as u8;
                }
                2 => {
                    let i = r.below(m.len() + 1);
                    m.insert(i, r.next() as u8);
                }
                _ if !m.is_empty() => {
                    let i = r.below(m.len());
                    m.remove(i);
                }
                _ => {}
            }
        }
        m
    }
    let v = read("codec.json");
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut loaded, mut refused, mut decoded, mut dec_refused) = (0, 0, 0, 0);
    for entry in v["schemas"].as_array().unwrap() {
        let d = unhex(s(entry, "descriptor"));
        let wires: Vec<Vec<u8>> = entry["cases"].as_array().unwrap().iter().map(|c| unhex(s(c, "wire"))).collect();
        for _ in 0..2000 {
            let md = mutate(&mut r, &d);
            let Ok(desc) = Descriptor::load(&md) else {
                refused += 1;
                continue;
            };
            loaded += 1;
            let _ = desc.names().count();
            for w in &wires {
                let mut rec = json::Recorder::default();
                match bitwire::decode(&desc, w, &mut rec) {
                    Ok(()) => decoded += 1,
                    Err(_) => dec_refused += 1,
                }
            }
        }
        let desc = Descriptor::load(&d).unwrap();
        let sh = shadow(&desc);
        for w in &wires {
            for _ in 0..300 {
                let mw = mutate(&mut r, w);
                match json::decode(&desc, &sh, &mw) {
                    Ok(_) => decoded += 1,
                    Err(_) => dec_refused += 1,
                }
            }
        }
    }
    println!(
        "mutations: descriptors {loaded} loaded / {refused} refused; messages {decoded} decoded / {dec_refused} refused; no panics"
    );
}

/// A sink error stops the decode and comes back unchanged.
#[test]
fn sink_error_propagates() {
    use bitwire::{Op, Sink};
    #[derive(Debug, PartialEq)]
    enum MyErr {
        Wire(Error),
        Stop,
    }
    impl From<Error> for MyErr {
        fn from(e: Error) -> Self {
            MyErr::Wire(e)
        }
    }
    struct StopAtUint;
    impl Sink for StopAtUint {
        type Error = MyErr;
        fn bool(&mut self, _: bool) -> Result<(), MyErr> { Ok(()) }
        fn uint(&mut self, _: Op, _: u64) -> Result<(), MyErr> { Err(MyErr::Stop) }
        fn int(&mut self, _: Op, _: i64) -> Result<(), MyErr> { Ok(()) }
        fn f32(&mut self, _: f32) -> Result<(), MyErr> { Ok(()) }
        fn f64(&mut self, _: f64) -> Result<(), MyErr> { Ok(()) }
        fn string(&mut self, _: &str) -> Result<(), MyErr> { Ok(()) }
        fn bytes(&mut self, _: &[u8]) -> Result<(), MyErr> { Ok(()) }
        fn enum_index(&mut self, _: u8) -> Result<(), MyErr> { Ok(()) }
        fn enter_object(&mut self) -> Result<(), MyErr> { Ok(()) }
        fn field(&mut self, _: u8, _: bool, _: bool) -> Result<(), MyErr> { Ok(()) }
        fn enter_array(&mut self, _: u32) -> Result<(), MyErr> { Ok(()) }
        fn variant(&mut self, _: u8) -> Result<(), MyErr> { Ok(()) }
        fn leave(&mut self) -> Result<(), MyErr> { Ok(()) }
    }
    let v = read("codec.json");
    let entry = v["schemas"].as_array().unwrap().iter().find(|e| e["name"] == "uint16").unwrap();
    let dbytes = unhex(s(entry, "descriptor"));
    let desc = Descriptor::load(&dbytes).unwrap();
    assert_eq!(bitwire::decode(&desc, &[0x05], &mut StopAtUint), Err(MyErr::Stop));
    assert_eq!(bitwire::decode(&desc, &[0x80], &mut StopAtUint), Err(MyErr::Wire(Error::Truncated)));
}
