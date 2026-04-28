#!/usr/bin/env python3

import os
import sys
import json
import base64
import hashlib
from Crypto.Cipher import AES

def generate_approval_link(run_id, step_id, action, inputs=None, base_url="http://localhost:3000"):
    # 1. Prepare payload
    payload = {
        "run_id": run_id,
        "step_id": step_id,
        "action": action,
        "inputs": inputs or {}
    }
    plaintext = json.dumps(payload).encode('utf-8')

    # 2. Derive key from JWT_SECRET using SHA256 (matches Rust implementation)
    # Default local dev secret in Stormchaser is b"stormchaser-secret-dev-only"
    secret = os.environ.get("JWT_SECRET", "stormchaser-secret-dev-only").encode('utf-8')
    key = hashlib.sha256(secret).digest()

    # 3. Encrypt with AES-GCM
    # Generate 12 byte nonce (IV)
    nonce = os.urandom(12)
    cipher = AES.new(key, AES.MODE_GCM, nonce=nonce)
    ciphertext, tag = cipher.encrypt_and_digest(plaintext)

    # 4. Construct token (Nonce + Ciphertext + Tag)
    # Note: In Rust's aes-gcm crate, the authentication tag is appended to the ciphertext
    encrypted_data = nonce + ciphertext + tag

    # 5. Base64 URL-safe encode (no padding)
    token = base64.urlsafe_b64encode(encrypted_data).decode('utf-8').rstrip('=')

    print(f"Payload: {json.dumps(payload)}")
    print(f"Generated Link: {base_url}/api/v1/approve-link/{token}")

if __name__ == "__main__":
    if len(sys.argv) < 4:
        print("Usage: ./generate-approval-link.py <run_id> <step_id> <approve|reject>")
        sys.exit(1)

    generate_approval_link(sys.argv[1], sys.argv[2], sys.argv[3])
