# A 10,000-share buy sweeping a deep book, replayed from an engine-matched
# recording.
# Re-record with `docs/media/render.sh sweep` from the repository root.
Output docs/media/sweep.raw.gif

Set Shell "bash"
Set FontSize 14
Set Width 1160
Set Height 560
Set Padding 16
Set Theme "GitHub Dark"
Set TypingSpeed 8ms

Hide
Type "export PS1='$ ' && cargo build -q --release -p examples-matching-engine && clear" Enter
Wait /^\$ ?$/
Show

Type "scripts/play-recording.sh web/scenarios/large_buy_sweep.txt --from 112 --to 176 -- --interval-secs 1 --tick-size 0.01 --window 30 --heatmap"
Sleep 600ms
Enter
Sleep 6.8s
Screenshot docs/media/sweep.raw.png
Sleep 2.5s
