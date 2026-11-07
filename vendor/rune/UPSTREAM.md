# Rune source

Imported from https://github.com/anomalyco/rune at
`ccb4823f55749dfa667891ef7f8427017e960056` (MIT).
Rune Kit and the supporting crates use the same immutable revision.

The standalone manifest replaces inherited workspace metadata and sibling paths.
Application-specific native extensions add Linux custom titlebars and interactive
resize, expose window geometry, and restore saved window state. Native resize
starts in the pointer-press event so Wayland retains the initiating input serial.
Windows retains Rune's native custom frame and caption behavior.

Import cleanup removes trailing spaces and surplus blank lines at EOF from
text files. License wording and precompiled shader bytes are unchanged.

Keep behavioral changes small and covered by tests; do not edit precompiled GPU
shaders. The application image compositor lives outside this vendor directory.
