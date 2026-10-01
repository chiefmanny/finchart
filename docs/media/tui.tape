# The interactive viewer: zoom out from 200 candles to all 5,000 and back. The
# status line shows display-resolution LOD (candles → bars) as the window grows.
# Re-record with `docs/media/render.sh tui` from the repository root.
Output docs/media/tui.raw.gif

Set Shell "bash"
Set FontSize 14
Set Width 1160
Set Height 560
Set Padding 16
Set Theme "GitHub Dark"
Set TypingSpeed 8ms

Hide
Type "export PS1='$ ' && cargo build -q --release -p examples-tui && clear" Enter
Wait /^\$ ?$/
Show

Type "cargo run -q --release -p examples-tui"
Sleep 500ms
Enter
Sleep 1.8s
Type@220ms "jjjjjjjjjjjjjjj"
Sleep 2.5s
Screenshot docs/media/tui.raw.png
Sleep 300ms
Type@220ms "kkkkkkkkkkkk"
Sleep 600ms
Type@250ms "hhhhhh"
Sleep 800ms
Type "r"
Sleep 1.5s
Type "q"
Sleep 300ms
