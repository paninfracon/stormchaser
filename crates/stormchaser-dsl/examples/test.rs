use stormchaser_dsl::StormchaserParser;

fn main() {
    let dsl = std::fs::read_to_string("tests/dogfood.storm").unwrap();
    let parser = StormchaserParser::new();
    let ast = parser.parse(&dsl).unwrap();
    println!("{:#?}", ast.storage);
}
