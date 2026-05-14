#!/bin/bash
TOKEN=$(python3 scripts/generate_dev_token.py)
curl -s -H "Authorization: Bearer $TOKEN" http://localhost:3000/api/v1/runs/85450307-e034-433e-870d-52d1604a9387/steps/d1584f2d-604a-4784-9391-184ebc961438/logs | jq -r '.[]' | tail -n 50
