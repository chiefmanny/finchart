# The synthetic live replay: seeded ticks fold into a forming candle, order flow
# and a Level 2 book in real time.
# Re-record with `docs/media/render.sh live` from the repository root.
Output docs/media/live.raw.gif

Set Shell "bash"
Set FontSize 14
Set Width 1160
Set Height 560
Set Padding 16
Set Theme "GitHub Dark"
Set TypingSpeed 8ms

Hide
Type "export PS1='$ ' && cargo build -q --release -p examples-live && clear" Enter
Wait /^\$ ?$/
Show

Type "cargo run -q --release -p examples-live -- --fps 12"
Sleep 600ms
Enter
Sleep 8s
Screenshot docs/media/live.raw.png
Sleep 1s
Type "q"
Sleep 300ms
