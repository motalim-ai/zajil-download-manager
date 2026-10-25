# Zajil interface strings in English (Fluent, https://projectfluent.org).
# Every key here exists in ar/zajil.ftl with the same variables, checked by
# crates/zajil-i18n/tests/translations.rs.
# Counted messages select on PLURAL($count), not on $count: PLURAL returns
# the CLDR category, the same in Rust and in the GUI (see ar/zajil.ftl).

app-name = Zajil

## Counts

downloads-count =
    { PLURAL($count) ->
        [one] { $count } download
       *[other] { $count } downloads
    }
connections-count =
    { PLURAL($count) ->
        [one] { $count } connection
       *[other] { $count } connections
    }
queues-count =
    { PLURAL($count) ->
        [one] { $count } queue
       *[other] { $count } queues
    }
files-count =
    { PLURAL($count) ->
        [one] { $count } file
       *[other] { $count } files
    }
seconds-left =
    { PLURAL($count) ->
        [one] { $count } second
       *[other] { $count } seconds
    }
minutes-left =
    { PLURAL($count) ->
        [one] { $count } minute
       *[other] { $count } minutes
    }
hours-left =
    { PLURAL($count) ->
        [one] { $count } hour
       *[other] { $count } hours
    }

## Units ($value is a number formatted beforehand)

size-bytes = { $value } B
size-kib = { $value } KiB
size-mib = { $value } MiB
size-gib = { $value } GiB
speed-per-second = { $size }/s
size-of-total = { $done } of { $total }

## Download states

status-waiting = Waiting
status-active = Downloading
status-paused = Paused
status-done = Done
status-failed = Failed
status-open-browser = Open the browser to go on
status-needs-ok = Waits for your OK
status-scheduled = Scheduled { $time }

## Sidebar

nav-all = All downloads
nav-active = Downloading
nav-done = Done
nav-categories = Categories
nav-queues = Queues
nav-settings = Settings
queue-window = from { $start } to { $stop }

## Built-in categories (key category-<id>)

category-programs = Programs
category-video = Videos
category-music = Audio
category-documents = Documents
category-archives = Archives
category-none = No category

## Paste bar and tools

paste-placeholder = Paste a link, or drag it here
paste-download = Download
action-resume = Resume
action-pause = Pause
action-delete = Delete
action-details = Details
action-open-folder = Open folder
action-cancel-download = Cancel download
search-placeholder = Search and filter

## Table

column-name = Name
column-size = Size
column-progress = Progress
column-speed = Speed
column-left = Left
column-status = Status
progress-label =
    { PLURAL($count) ->
        [one] { $percent } in { $count } part
       *[other] { $percent } in { $count } parts
    }

## Footer

footer-active =
    { PLURAL($count) ->
        [one] Downloading: { $count } download
       *[other] Downloading: { $count } downloads
    }
footer-total-speed = Total speed { $speed }
footer-limit-none = Speed limit: none
footer-limit = Speed limit: { $speed }
footer-waiting =
    { PLURAL($count) ->
        [one] Waiting: { $count } download
       *[other] Waiting: { $count } downloads
    }

## Add dialog

add-title = Add a download
add-url = Link
add-probe-ranges = { $size }. The server allows ranges, so it can download over several connections and resume later.
add-probe-single = { $size }. The server does not allow ranges, so it downloads over one connection.
add-probe-unsure = { $size }. The server allows ranges but gives no size, or no way to be sure the file has not changed, so it downloads over one connection and starts over if cut.
add-file-name = File name
add-category = Category
add-category-extension = { $category } (from the extension { $extension })
add-category-site = { $category } (from the site { $site })
add-queue = Queue
add-folder = Folder
add-change = Change
add-connections = Connections
add-connections-hint = { $min } to { $max }
add-limit = Speed limit for this download
add-limit-none = None
add-cancel = Cancel
add-later = Download later
add-start = Start download

## Details

details-title = Download details
details-from-to = From { $site }, to { $folder }
details-file-map = File map: each square is { $size }
details-map-done = Arrived
details-map-writing = Being written
details-map-split = Split point
details-part = Part { $number }
details-part-done = Done
details-speed = Speed
details-left = Left
details-parts = Parts
details-connections-value = { $active } of { $total } working
details-resumable = Can be resumed
details-yes = Yes
details-no = No
details-unknown = Unknown
details-back = Back to the list
details-limit = Speed limit
details-queue = Queue
details-speed-chart = Speed over the last minute
details-chart-then = A minute ago
details-chart-now = Now
details-failed = The download stopped: { $error }
details-open-browser = This download came from the browser with its cookies, and Zajil does not keep cookies. Open the browser (and the same profile) it came from: Zajil takes them from it and goes on by itself. If that browser is gone, go on without the cookies.
details-without-cookies = Go on without cookies
details-needs-ok = This file can run programs on your computer (a program, an installer, a script or an Office file with macros), and it came from a page in the browser, so it will not download until you start it. Start it only if you trust the site.

## Refresh the download link

refresh-action = Refresh link
refresh-title = Refresh the download link
refresh-hint = If the link expired, get a new link to the same file from the site and paste it here. Zajil makes sure it is the same file, then goes on from where it stopped.
refresh-url = New link
refresh-cancel = Cancel
refresh-go = Refresh and go on
refresh-not-stopped = Pause the download first, then refresh its link.
refresh-stopping = The download is still stopping. Wait a moment, then try again.
refresh-busy = This download's link is already being refreshed.
refresh-cancelled = The refresh was given up; the download did not change.
refresh-done = This download has finished: it needs no new link.
refresh-bad-link = Zajil does not take this link: it must start with http or https, without a user name or password.
refresh-size = The new link is to another file: its size is { $theirs }, this download's { $ours }.
refresh-size-unknown = The size of the new file or of this download is unknown, so Zajil cannot be sure it is the same file.
refresh-version = The new link is to another version of the file, so Zajil will not go on from it, not to spoil the file.
refresh-validator-kind = The server describes the new file differently, so Zajil cannot be sure it is the same file.
refresh-new-no-validator = The new link's server gives nothing Zajil can check that it is the same file with.
refresh-no-validator = This download's server gave nothing Zajil can check the file with, so its link cannot be refreshed. Start a new download from the new link.
refresh-failed = The link was not refreshed.

## Settings

settings-title = Settings
settings-applied = Every change applies at once.
settings-language = Language
settings-language-hint = The interface changes at once, without a restart.
settings-theme = Appearance
settings-theme-hint = System follows the computer's setting.
settings-theme-system = System
settings-theme-light = Light
settings-theme-dark = Dark
settings-digits = Digits
settings-digits-hint = In sizes, speeds and times.
settings-folder = Download folder
settings-folder-hint = Each category can have its own folder too.
settings-connections = Connections per download
settings-connections-hint = { $default } by default, at most { $max }.
settings-limit = Speed limit for the whole program
settings-limit-hint = For all downloads together.
settings-limit-none = None
settings-limit-set = Set a speed
settings-autostart = When the computer starts
settings-autostart-label = Start Zajil with the computer
settings-notifications = Notifications
settings-notifications-label = Tell me when a download finishes or fails
settings-browser = Browser
settings-browser-hint = Zajil registers itself with Chrome, Edge, Chromium and Firefox at each start, installed or not, so its extension can reach it. This button registers again and says how it went.
settings-browser-fix = Fix the browser integration
settings-browser-ok = { $browser }: registered
settings-browser-failed = { $browser }: not registered ({ $error })
settings-browser-missing = zajil-host was not found next to the program, so no browser can reach Zajil.
settings-browser-temporary = The program runs from a place that goes away when it closes (an AppImage, or macOS moving it aside), so no browser was registered. Install Zajil in a lasting place.
settings-browser-unsafe = Another user could change zajil-host or its folder, so no browser was registered.
settings-browser-working = Registering…
settings-clipboard = Clipboard
settings-clipboard-label = Watch the clipboard and put the download links I copy in the add box
settings-clipboard-hint = Zajil looks at the clipboard twice a second for a link to a file, and keeps nothing of it. It does not contact the link or add anything before you press OK in «Add a link».
settings-export = Export queues
settings-import = Import queues
settings-export-warning = The file holds the links as they are, and some may contain temporary access keys. Do not share it.

## Categories

categories-title = Categories
categories-hint = A download's category comes from its site first, then from its file extension.
categories-new = New category
categories-name = Name
categories-extensions = Extensions
categories-extensions-hint = Type an extension and press Enter.
categories-add-extension = extension
categories-sites = Sites
categories-sites-hint = Everything from these sites goes here.
categories-folder = Folder
categories-folder-none = The folder each download is given
categories-try = Try
categories-try-hint = Type a file name to see its category.
categories-delete = Delete category

## System tray

tray-open = Open Zajil
tray-pause-all = Pause all
tray-resume-all = Resume all
tray-quit = Quit Zajil

## Notifications

notify-done-title = Download finished
notify-done-body = { $name }
notify-failed-title = Download failed
notify-failed-body = { $name }: { $reason }

## Settings and categories (keys that were missing: review of #57)
settings-queues = Queues
settings-folder-choose = Choose a folder
settings-folder-default = The system's downloads folder
settings-import-keep-folders = Use the folders in the imported file
settings-limit-value = Limit
settings-limit-unit = Unit
unit-kib-per-second = KiB/s
unit-mib-per-second = MiB/s
export-done = The queues were exported.
import-done =
    { PLURAL($count) ->
        [one] { $count } download added.
       *[other] { $count } downloads added.
    }
import-kept = These queues kept their own settings: { $names }.
categories-add-site = Add a site
categories-folder-clear = No folder
categories-remove = Remove { $value }

## Main window (M5.5, the design of #83)

settings-edit-categories = Edit the categories
tb-add = Add link
tb-resume = Resume
tb-pause = Pause
tb-pause-all = Pause all
tb-delete = Delete
tb-delete-done = Clear finished
tb-settings = Settings
tb-schedule = Schedule
tb-run-queue = Run queue
tb-halt-queue = Halt queue
tb-toolbar = Download tools
tree-unfinished = Unfinished
tree-finished = Finished
tree-label = Downloads, categories and queues
confirm-remove = Take off the list
confirm-cancel = Cancel
confirm-delete-done =
    { PLURAL($count) ->
        [one] { $count } finished download shown is taken off the list. The file itself stays where it is.
       *[other] { $count } finished downloads shown are taken off the list. The files themselves stay where they are.
    }
confirm-delete =
    { PLURAL($count) ->
        [one] { $count } download is taken off the list. What was downloaded stays on the disk.
       *[other] { $count } downloads are taken off the list. What was downloaded stays on the disk.
    }
tree-expand = Show what is under it
queue-main = Main queue
queue-starts = Starts { $time }
col-name = File name
col-size = Size
col-status = Status
col-left = Time left
col-speed = Speed
col-last-try = Last try
col-description = Description
st-in-queue = In queue
st-paused = Paused
st-error = Error { $code }
st-failed = Failed
footer-selected = Selected: { $number }
addlink-title = Add a link
addlink-url = Link
addlink-from-clipboard = Taken from the clipboard
addlink-ok = OK
addlink-cancel = Cancel

## M5.5: the menus, the right-click menu and the columns

menu-bar = Menus
menu-file = File
menu-edit = Edit
menu-downloads = Downloads
menu-view = View
menu-help = Help
cmd-add = Add link…
cmd-import = Import queues…
cmd-export = Export queues…
cmd-quit = Quit
cmd-select-all = Select all
cmd-find = Find
cmd-copy-link = Copy link
cmd-delete-done = Delete finished
cmd-resume = Resume
cmd-pause = Pause
cmd-pause-all = Pause all
cmd-delete = Delete…
cmd-run-queue = Start queue
cmd-halt-queue = Stop queue
cmd-schedule = Schedule…
cmd-settings = Settings…
cmd-categories = Categories…
cmd-columns = Columns
cmd-open = Open
cmd-open-folder = Open folder
cmd-progress = Progress window
cmd-refresh-link = Refresh link…
cmd-properties = Properties
cmd-about = About Zajil
cmd-move-to-queue = Move to queue
about-text = Zajil, version { $version }
table-label = Downloads
col-resize = { $column } column width
sys-hide = Hide { $app }
sys-hide-others = Hide Others
sys-show-all = Show All
sys-undo = Undo
sys-redo = Redo
sys-cut = Cut
sys-copy = Copy
sys-paste = Paste
sys-window = Window
sys-minimize = Minimize
sys-zoom = Zoom
row-menu = Download menu
open-code-question = “{ $name }” is a program, or a file that can run commands on your computer. Open it?
open-code-ok = Open

## M5.5: the add windows
info-title = Download file info
add-new-category = New category
add-remember = Remember this folder for “{ $category }”
add-resumable = Can be resumed
add-not-resumable = Can't be resumed
add-later-queue = Download later in a queue
add-checking = Asking the server about the file…
add-about-file = About the file
addlink-info-open = «Download file info» is open for another link. Finish or cancel it first.
add-remember-failed = The download was added, but the folder wasn't kept for the category: { $error }

## Quit with downloads running (owner's decision on #83)
quit-question =
    { PLURAL($count) ->
        [one] { $count } download is running. Quit Zajil?
       *[other] { $count } downloads are running. Quit Zajil?
    }
quit-ok = Quit Zajil

## Video (M6)

video-soon = Video downloads are coming soon.

## The main window's title bar (the design of #83)
app-title = Zajil Download Manager
win-minimize = Minimize
win-maximize = Maximize
win-restore = Restore
win-close = Close
