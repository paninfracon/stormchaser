import re

file_path = "crates/stormchaser-api/src/routes/mod.rs"
with open(file_path, "r") as f:
    content = f.read()

# Add ToSchema to derives
content = re.sub(r'#\[derive\(([^)]*)\)\]\n/// (Directrunrequest|Createwebhookrequest|Createeventrulerequest|Createcronworkflowrequest|Createstoragebackendrequest|Cronworkflowresponse)\.',
                 lambda m: f'#[derive({m.group(1)}, ToSchema)]\n/// {m.group(2)}.' if 'ToSchema' not in m.group(1) else m.group(0),
                 content)

# Add #[schema(value_type = Object)] to Value fields
content = re.sub(r'(\s+)pub inputs: Value,', r'\1#[schema(value_type = Object)]\1pub inputs: Value,', content)
content = re.sub(r'(\s+)pub config: Value,', r'\1#[schema(value_type = Object)]\1pub config: Value,', content)
content = re.sub(r'(\s+)pub config: Option<Value>,', r'\1#[schema(value_type = Object)]\1pub config: Option<Value>,', content)

with open(file_path, "w") as f:
    f.write(content)
