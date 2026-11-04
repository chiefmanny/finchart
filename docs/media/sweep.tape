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
Type `export PS1='$ ' PATH="$PWD/target/release:$PATH" && clear` Enter
Wait /^\$ ?$/
Show

Type "finchart replay web/scenarios/large_buy_sweep.txt --start 0:56 --speed 4"
Sleep 600ms
Enter
# Play through the sweep (~1:11), rewind 10 s, watch it again, then pause.
Sleep 6s
Left
Sleep 150ms
Left
Sleep 3.5s
Space
Sleep 1.5s
Screenshot docs/media/sweep.raw.png
Sleep 500ms
Hide
Type "q"
