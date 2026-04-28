import base64
import json
import hmac
import hashlib
import time

def b64url(b):
    return base64.urlsafe_b64encode(b).decode('utf-8').rstrip('=')

header = {"alg": "HS256", "typ": "JWT"}
payload = {
    "sub": "stormchaser-admin",
    "email": "stormchaser-admin@paninfracon.net",
    "exp": int(time.time()) + 3600
}

h_b64 = b64url(json.dumps(header, separators=(',', ':')).encode('utf-8'))
p_b64 = b64url(json.dumps(payload, separators=(',', ':')).encode('utf-8'))

msg = f"{h_b64}.{p_b64}"
sig = hmac.new(b"stormchaser-secret-dev-only", msg.encode('utf-8'), hashlib.sha256).digest()
sig_b64 = b64url(sig)

print(f"{msg}.{sig_b64}")
