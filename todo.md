#TODO

##General
    - [x] Conformation when a user wants to kill the app, but there are unsaved changes
    - [x] Conformation when a user wants to delete a track which has content.
    - I want to create presets for track (bas guitar, rhythm guitar, solo, etc) we should have user presets, and a number of default presets.

##Arrange tab
    -We need a place for global tracks like tempo track, and signature changes, my idea would an collapsable area between the normal tracks and the time indication
    -Implement tempo track and signature track
    - [x] Selecting a track should highlight it
    - [x] Move the delete track button to the top right position. Its different from the rest of the buttons
    -The solo functionality does not seem to work.

##Mix tab
    -We need a new solution for subtracks. My suggestion would be to make the collapsed view the default. But then make it a bit wider (maybe two slots, and show db meters of all subtracks. When expendanded the user can modify gain etc.
    - [x] Something goes wrong when saving with busses. When opening a saved project the bus is there, but does not work. When removing the bus, audio can be heard again.

##Compose tab
    - [x] We need a solution about the editing of instruments, the current view is to small so its hard to pick the correct notes. We need to brainstorm about this.
    

##Plugins general
    - We need better control, maybe knobs are better then slider.

##Delay plugin
    - [x] Feedback range is to large afaik


##Drum plugin
    - [x] We should be able to download the drumkit as a zip from a server. (see https://resonance.plok.org/index.json)
    - [x] Check if round robin is executed correctly (also add unit tests)
    - Implement all pads/parts found in /home/jorrit/Documents/Guitar/drummica
