import os
import stat
import secrets
import sys
try:
    from passlib.hash import bcrypt
except ImportError:
    print("\033[0;31mError: passlib is not installed. Please pip install passlib[bcrypt].\033[0m", file=sys.stderr)
    exit(1)

def gen_and_hash():
    pw = secrets.token_urlsafe(16)
    return pw, bcrypt.hash(pw)

repo_root = os.environ.get("REPO_ROOT", ".")
db_password = os.environ.get("STORMCHASER_DEV_PASSWORD", "")
if not db_password:
    print("\033[0;31mError: STORMCHASER_DEV_PASSWORD is not set.\033[0m", file=sys.stderr)
    sys.exit(1)

template_path = os.path.join(repo_root, "deploy/dex/config.yaml")
out_path = os.path.join(repo_root, "deploy/dex/config.generated.yaml")
cred_path = os.path.join(repo_root, "deploy/dex/credentials.generated")
role_map = {"ADMIN": "admin", "DEV": "dev", "OPS": "ops", "SEC": "sec"}

with open(template_path, "r") as f:
    content = f.read()

client_secret = os.environ.get("STORMCHASER_CLI_SECRET", "stormchaser-cli-secret")
content = content.replace("DEX_DB_PASSWORD", db_password)
content = content.replace("DEX_CLIENT_SECRET", client_secret)

with open(cred_path, "w") as cred_file:
    cred_file.write("# Dex persona credentials — keep secret, do not commit\n")
    cred_file.write(f"dex-client-secret: {client_secret}\n")
    for role, role_email in role_map.items():
        pw, phash = gen_and_hash()
        cred_file.write(f"stormchaser-{role_email}@paninfracon.net: {pw}\n")
        content = content.replace(f"PASSWORD_HASH_{role}", phash)

# Verify all placeholders were replaced
remaining = [line for line in content.splitlines() if "PASSWORD_HASH_" in line or "DEX_DB_PASSWORD" in line or "DEX_CLIENT_SECRET" in line]
if remaining:
    print(f"\033[0;31mError: unreplaced placeholders found in generated config:\033[0m", file=sys.stderr)
    for line in remaining:
        print(f"  {line.strip()}", file=sys.stderr)
    sys.exit(1)

os.chmod(cred_path, stat.S_IRUSR | stat.S_IWUSR)

with open(out_path, "w") as f:
    f.write(content)

os.chmod(out_path, stat.S_IRUSR | stat.S_IWUSR)

print(f"Dex credentials written to {cred_path} (mode 0600).")
print(f"Dex config written to {out_path} (mode 0600).")
