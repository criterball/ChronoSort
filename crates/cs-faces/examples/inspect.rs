//! Print the inputs/outputs of an ONNX model: cargo run -p cs-faces --example inspect -- model.onnx
use tract_onnx::prelude::*;
fn main() -> TractResult<()> {
    let path = std::env::args().nth(1).expect("model path");
    let m = tract_onnx::onnx().model_for_path(&path)?;
    for (i, o) in m.input_outlets()?.iter().enumerate() {
        println!("input {i}: {} {:?}", m.node(o.node).name, m.outlet_fact(*o)?);
    }
    for (i, o) in m.output_outlets()?.iter().enumerate() {
        println!("output {i}: {} {:?}", m.node(o.node).name, m.outlet_fact(*o)?);
    }
    Ok(())
}
