# Liquidity withdraws, then aggressive buying runs through the gap — replayed
# from an engine-matched recording.
# Re-record with `docs/media/render.sh crisis` from the repository root.
Output docs/media/crisis.raw.gif

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

Type "scripts/play-recording.sh web/scenarios/liquidity_crisis.txt --from 262 --to 326 -- --interval-secs 1 --tick-size 0.01 --window 48 --heatmap"
Sleep 600ms
Enter
Sleep 6.8s
Screenshot docs/media/crisis.raw.png
Sleep 2.5s
