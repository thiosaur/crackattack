# Layout of the macOS disk image, read by dmgbuild (https://dmgbuild.readthedocs.io):
#   dmgbuild -s packaging/macos/dmg-settings.py -D app="path/Crack Attack.app" \
#            -D dir=packaging/macos "Crack Attack" out.dmg
# dmgbuild writes the window layout itself (no Finder scripting), so it works on a CI runner.
import os

# `defines` is injected by dmgbuild: `app` is the .app to ship, `dir` the folder of this file.
app = defines["app"]  # noqa: F821
here = defines["dir"]  # noqa: F821

format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}
# The 2x image next to this one is combined with it into a Retina background.
background = os.path.join(here, "dmg-background.png")
window_rect = ((200, 120), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 128
text_size = 13
icon_locations = {
    os.path.basename(app): (170, 190),
    "Applications": (490, 190),
}
