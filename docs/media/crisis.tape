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
Type `export PS1='$ ' PATH="$PWD/target/release:$PATH" && clear` Enter
Wait /^\$ ?$/
Show

Type "finchart replay web/scenarios/liquidity_crisis.txt --start 2:11 --speed 4"
Sleep 600ms
Enter
Sleep 8s
Space
Sleep 1.5s
Screenshot docs/media/crisis.raw.png
Sleep 500ms
Hide
Type "q"
