Output "walkthrough.gif"
Set FontSize 14
Set Width 1200
Set Height 800
Set TypingSpeed 150ms

Type "cd tests/demo"
Enter
Sleep 1s

Type "STORMCHASER_TOKEN=$(cat /tmp/.token) ../../target/debug/stormchaser-tui"
Enter
Sleep 4s

Type "r"
Sleep 1s

Down 1
Sleep 500ms
Enter
Sleep 2s

Type "demo-user"
Sleep 500ms
Tab
Sleep 500ms

Type "25"
Sleep 500ms
Tab
Sleep 500ms

Down
Sleep 500ms
Down
Sleep 2s
Tab
Sleep 500ms

Down
Sleep 2s
Tab
Sleep 500ms

Down
Sleep 500ms
Tab
Sleep 500ms

Type "false"
Sleep 500ms
Tab
Sleep 500ms

Type "testuser"
Sleep 500ms

Enter
Sleep 1s

Enter
Sleep 11s

Tab
Sleep 2s
Type "}}}}}}}}"
Sleep 6s
Type "}}}}}}}}"
Sleep 4s
