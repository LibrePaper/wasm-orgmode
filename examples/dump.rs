use std::io::Read;
fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    let org = wasm_orgmode::Org::parse(&s);
    println!("{:#?}", wasm_orgmode::SyntaxNode::new_root(org.green().clone()));
}
