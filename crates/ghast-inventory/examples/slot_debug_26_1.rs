//! Lab tool: decodes a dumped 26.1 window packet body slot by slot and component by component,
//! naming the first component the generated layout cannot read.
//!
//! ```text
//! cargo run -p ghast-inventory --example slot_debug_26_1 -- <0x12|0x14> body.bin
//! ```

use bytes::Bytes;
use ghast_inventory::v26_1::COMPONENT_NAMES;
use ghast_protocol::Decode;
use ghast_protocol::codec::Reader;
use ghast_protocol::defs::p775::types::SlotComponent;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = i32::from_str_radix(args[0].trim_start_matches("0x"), 16).unwrap();
    let body = Bytes::from(std::fs::read(&args[1]).unwrap());
    let mut r = Reader::new(&body);
    r.read_varint().unwrap();
    r.read_varint().unwrap();
    let n = if id == 0x12 {
        r.read_varint().unwrap() + 1
    } else {
        r.read_i16().unwrap();
        1
    };
    for k in 0..n {
        let count = r.read_varint().unwrap();
        if count <= 0 {
            continue;
        }
        let item = r.read_varint().unwrap();
        let added = r.read_varint().unwrap();
        let removed = r.read_varint().unwrap();
        for _ in 0..added {
            let at = r.position();
            let ty = r.clone().read_varint().unwrap();
            let name = COMPONENT_NAMES.get(ty as usize).copied().unwrap_or("?");
            match SlotComponent::decode(&mut r) {
                Ok(_) => println!(
                    "slot {k} item {item}: {name} ok ({} bytes)",
                    r.position() - at
                ),
                Err(e) => {
                    println!(
                        "slot {k} item {item}: {name} FAILS: {e}; bytes {:02x?}",
                        &body[at..body.len().min(at + 48)]
                    );
                    return;
                }
            }
        }
        for _ in 0..removed {
            r.read_varint().unwrap();
        }
    }
    println!("all slots decode");
}
