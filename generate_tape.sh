#!/bin/bash
python3 scripts/generate_dev_token.py > /tmp/.token
bash complete_run.sh &
docker run --rm -v $PWD:/workspace -w /workspace -v /tmp/.token:/tmp/.token --network host ghcr.io/charmbracelet/vhs walkthrough.tape
rm -f /tmp/.token