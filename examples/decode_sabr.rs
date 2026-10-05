use std::fs;
use ump_stream::part::decode_all;
use ump_stream::proto::Message;

fn main() {
    let req = fs::read("tests/fixtures/real/req_001.bin").expect("req fixture");
    println!("request: {} bytes", req.len());
    match Message::decode(&req) {
        Ok(msg) => {
            for field in &msg.fields {
                println!("  field {}: {:?}", field.number, field.value);
            }
        }
        Err(e) => println!("  decode error: {}", e),
    }

    let resp = fs::read("tests/fixtures/real/resp_001.bin").expect("resp fixture");
    println!("\nresponse: {} bytes", resp.len());
    match decode_all(&resp) {
        Ok(parts) => {
            for part in &parts {
                println!("  part kind={} size={}", part.kind, part.payload.len());
                match part.message() {
                    Ok(msg) => {
                        for field in &msg.fields {
                            println!("    field {}: {:?}", field.number, field.value);
                        }
                    }
                    Err(e) => println!("    (not a message: {})", e),
                }
            }
        }
        Err(e) => println!("  decode error: {}", e),
    }
}
