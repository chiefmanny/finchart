# `finchart replay` with the crosshair: read out the sweep bar, then step back
# through the quiet seconds before it. (Mouse hover does the same; VHS types keys.)
# Re-record with `docs/media/render.sh crosshair` from the repository root.
Output docs/media/crosshair.raw.gif

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

Type "finchart replay web/scenarios/large_buy_sweep.txt --start 1:12 --paused"
Sleep 500ms
Enter
Sleep 1.2s
Type "c"
Sleep 900ms
Type ","
Sleep 2.2s
Screenshot docs/media/crosshair.raw.png
Type@450ms ",,,,,,"
Sleep 900ms
Type@250ms "......"
Sleep 1.5s
Escape
Sleep 1s
Hide
Type "q"
