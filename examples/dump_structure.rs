use std::fs;
use ump_stream::proto::{Message, Value};

fn print_message(msg: &Message, indent: usize) {
    for field in &msg.fields {
        let pad = "  ".repeat(indent);
        match &field.value {
            Value::Varint(v) => println!("{}field {}: varint {}", pad, field.number, v),
            Value::Fixed32(v) => println!("{}field {}: fixed32 {}", pad, field.number, v),
            Value::Fixed64(v) => println!("{}field {}: fixed64 {}", pad, field.number, v),
            Value::Bytes(b) => {
                if let Ok(sub) = Message::decode(b) {
                    println!("{}field {}: message ({} bytes)", pad, field.number, b.len());
                    print_message(&sub, indent + 1);
                } else if let Ok(s) = std::str::from_utf8(b) {
                    println!("{}field {}: string {:?}", pad, field.number, s);
                } else {
                    println!("{}field {}: bytes ({} bytes)", pad, field.number, b.len());
                }
            }
        }
    }
}

fn main() {
    let req = fs::read("tests/fixtures/real/req_001.bin").expect("req fixture");
    println!("=== SABR request structure ({} bytes) ===", req.len());
    match Message::decode(&req) {
        Ok(msg) => print_message(&msg, 0),
        Err(e) => println!("decode error: {}", e),
    }
}
