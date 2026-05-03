#!/bin/bash

# 1. Ensure Ollama has your coding model
echo "Checking Ollama for Qwen3-Coder..."
ollama pull qwen2.5-coder:32b-instruct-q4_K_M

# 2. Run the Gemini CLI automated router setup
# This downloads the specialized 1GB gemma3-1b-gpu-custom model and the LiteRT server
echo "Setting up Gemini Local Router..."
gemini gemma setup

# 3. Start the router service in the background (if not already running)
gemini gemma start

echo "Local environment ready."
