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
category-music = Music
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
search-placeholder = Search downloads

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
add-later = Add to queue
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
settings-clipboard-hint = Zajil looks at the clipboard twice a second for a link to a file, and keeps nothing of it. It does not contact the link or add anything before you press Download.
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
