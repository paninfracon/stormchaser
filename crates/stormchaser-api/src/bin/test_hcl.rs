use hcl::expr::*;

fn extract_vars(expr: &Expression, deps: &mut std::collections::HashSet<String>) {
    match expr {
        Expression::Traversal(t) => {
            if let Expression::Variable(var) = &t.expr {
                if var.as_str() == "inputs" {
                    if let Some(TraversalOperator::GetAttr(attr)) = t.operators.first() {
                        deps.insert(attr.as_str().to_string());
                    }
                }
            }
        }
        Expression::FuncCall(f) => {
            for arg in &f.args {
                extract_vars(arg, deps);
            }
        }
        Expression::Array(arr) => {
            for e in arr {
                extract_vars(e, deps);
            }
        }
        Expression::Object(obj) => {
            for (k, v) in obj {
                if let ObjectKey::Expression(e) = k {
                    extract_vars(e, deps);
                }
                extract_vars(v, deps);
            }
        }
        Expression::Parenthesis(p) => {
            extract_vars(p, deps);
        }
        Expression::Conditional(c) => {
            extract_vars(&c.cond_expr, deps);
            extract_vars(&c.true_expr, deps);
            extract_vars(&c.false_expr, deps);
        }
        Expression::Operation(op) => match &**op {
            Operation::Unary(u) => {
                extract_vars(&u.expr, deps);
            }
            Operation::Binary(b) => {
                extract_vars(&b.lhs_expr, deps);
                extract_vars(&b.rhs_expr, deps);
            }
        },
        Expression::ForExpr(f) => {
            extract_vars(&f.collection_expr, deps);
            extract_vars(&f.value_expr, deps);
            if let Some(k) = &f.key_expr {
                extract_vars(k, deps);
            }
            if let Some(c) = &f.cond_expr {
                extract_vars(c, deps);
            }
        }
        _ => {}
    }
}

fn extract_template_deps(query: &str) -> Vec<String> {
    let mut deps = std::collections::HashSet::new();
    if let Ok(template) = query.parse::<hcl::Template>() {
        for el in template.elements() {
            match el {
                hcl::template::Element::Interpolation(interp) => {
                    extract_vars(&interp.expr, &mut deps);
                }
                hcl::template::Element::Directive(dir) => match &**dir {
                    hcl::template::Directive::If(i) => {
                        extract_vars(&i.cond_expr, &mut deps);
                        for e in i.true_template.elements() {
                            if let hcl::template::Element::Interpolation(interp) = e {
                                extract_vars(&interp.expr, &mut deps);
                            }
                        }
                        if let Some(f) = &i.false_template {
                            for e in f.elements() {
                                if let hcl::template::Element::Interpolation(interp) = e {
                                    extract_vars(&interp.expr, &mut deps);
                                }
                            }
                        }
                    }
                    hcl::template::Directive::For(f) => {
                        extract_vars(&f.collection_expr, &mut deps);
                        for e in f.template.elements() {
                            if let hcl::template::Element::Interpolation(interp) = e {
                                extract_vars(&interp.expr, &mut deps);
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }
    let mut result: Vec<String> = deps.into_iter().collect();
    result.sort();
    result
}

fn main() {
    let deps = extract_template_deps(
        "sql://SELECT ${inputs.env} UNION %{ if inputs.prod } prod %{ endif }",
    );
    println!("{:?}", deps);
}
