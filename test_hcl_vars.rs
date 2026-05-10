fn main() {
    let expr_str = "sql://SELECT ${inputs.env} UNION ${inputs.x}";
    let template: hcl::Template = expr_str.parse().unwrap();
    println!("{:?}", template);
}
