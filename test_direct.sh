#!/bin/bash
TOKEN=$(python3 scripts/generate_dev_token.py)
PAYLOAD='{
  "dsl": "workflow \"complex-input-form\" {\n  description = \"A complex input form to showcase query and schema capabilities.\"\n\n  query \"roles_query\" {\n    type = \"mock\"\n    params = {\n      items = \"admin,developer,viewer\"\n    }\n  }\n\n  query \"regions_query\" {\n    type = \"mock\"\n    params = {\n      items = \"us-east-1,us-west-2,eu-central-1\"\n    }\n  }\n\n  query \"clusters_query\" {\n    type = \"mock\"\n    params = {\n      items = \"${inputs.region}-cluster-1,${inputs.region}-cluster-2\"\n    }\n  }\n\n  inputs {\n    type = \"object\"\n    properties {\n      username {\n        type    = \"string\"\n        pattern = \"^[a-zA-Z0-9_-]{3,16}$\"\n      }\n      age {\n        type    = \"integer\"\n        minimum = 18\n        maximum = 100\n      }\n      role {\n        type = \"string\"\n        enum = \"${queries.roles_query}\"\n      }\n      region {\n        type = \"string\"\n        enum = \"${queries.regions_query}\"\n      }\n      cluster {\n        type = \"string\"\n        enum = \"${queries.clusters_query}\"\n      }\n      dry_run {\n        type = \"boolean\"\n        default = true\n      }\n    }\n    allOf = [\n      {\n        if = { properties = { role = { const = \"developer\" } } }\n        then = { properties = { github_username = { type = \"string\" } }, required = [\"github_username\"] }\n      }\n    ]\n    required = [\"username\", \"role\", \"region\", \"cluster\"]\n  }\n\n  steps {\n    step \"echo-inputs\" \"JinjaRender\" {\n      spec {\n        template = <<EOF\n{\n  \"received_inputs\": {{ inputs | tojson }},\n  \"message\": \"Hello {{ inputs.username }}, deploying to {{ inputs.region }} cluster {{ inputs.cluster }}\"\n}\nEOF\n        output_key = \"echoed_data\"\n      }\n    }\n  }\n}\n",
  "inputs": {
    "username": "demo-user",
    "age": 25,
    "role": "admin",
    "region": "us-east-1",
    "cluster": "us-east-1-cluster-1",
    "dry_run": false
  }
}'

curl -s -X POST "http://localhost:3000/api/v1/runs/direct" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$PAYLOAD"
