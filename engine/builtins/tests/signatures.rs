//! Every implementation's signature and leaf counts must be exactly what
//! the compiler derives from its declaration; otherwise programs wouldn't link.

mod common;

use compiler::schema::parse_interface;
use compiler::types::leaves;
use serde_json::json;

#[test]
fn implementations_match_declarations() {
    let iface = parse_interface(&json!({"inputs": [], "output": {"type": "number"}, "builtins": common::declarations()})).unwrap();
    assert_eq!(iface.builtins.len(), sailing::BUILTINS.len(), "one implementation per declaration");
    for d in &iface.builtins {
        let f = sailing::BUILTINS
            .iter()
            .find(|f| f.signature == d.signature)
            .unwrap_or_else(|| panic!("no implementation with signature {}", d.signature));
        let args: u16 = d.params.iter().map(|(_, t, _)| leaves(t)).sum();
        assert_eq!((f.args, f.results), (args, leaves(&d.result)), "{}", d.signature);
    }
}
