Output "walkthrough-debug.gif"
Set FontSize 14
Set Width 1200
Set Height 800
Set TypingSpeed 50ms

Type "export STORMCHASER_TOKEN=$(python3 scripts/generate_dev_token.py)"
Enter
Sleep 1s
Type "echo TOKEN_IS: $STORMCHASER_TOKEN > debug.txt"
Enter
Sleep 1s
Type "curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/api/v1/runs >> debug.txt"
Enter
Sleep 2s
