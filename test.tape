Output "test.gif"
Set FontSize 14
Set Width 1200
Set Height 800
Set TypingSpeed 50ms

Type "cd tests/demo"
Enter
Sleep 1s

Type "STORMCHASER_TOKEN=$(cat /tmp/.token) ../../target/debug/stormchaser-tui > tui.out"
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
Sleep 4s
