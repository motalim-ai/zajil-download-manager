# نصوص واجهة زاجِل بالعربية (Fluent، https://projectfluent.org).
# كل مفتاح هنا موجود في en/zajil.ftl بنفس المتغيرات، ويتأكد من ذلك
# اختبار crates/zajil-i18n/tests/translations.rs.
# صيغ الجمع العربية الست حسب CLDR: zero وone وtwo وfew وmany وother.
# الاختيار دائمًا بـ PLURAL($count) لا بـ $count مباشرة: PLURAL يعيد اسم
# الفئة من CLDR، في Rust (ICU4X) وفي الواجهة (Intl.PluralRules). مكتبة
# الجمع التي يستعملها fluent-bundle في Rust تخطئ في 103 و111 بالعربية.

app-name = زاجِل

## الأعداد

downloads-count =
    { PLURAL($count) ->
        [zero] لا تحميلات
        [one] تحميل واحد
        [two] تحميلان
        [few] { $count } تحميلات
        [many] { $count } تحميلًا
       *[other] { $count } تحميل
    }
connections-count =
    { PLURAL($count) ->
        [zero] لا اتصالات
        [one] اتصال واحد
        [two] اتصالان
        [few] { $count } اتصالات
        [many] { $count } اتصالًا
       *[other] { $count } اتصال
    }
queues-count =
    { PLURAL($count) ->
        [zero] لا طوابير
        [one] طابور واحد
        [two] طابوران
        [few] { $count } طوابير
        [many] { $count } طابورًا
       *[other] { $count } طابور
    }
files-count =
    { PLURAL($count) ->
        [zero] لا ملفات
        [one] ملف واحد
        [two] ملفان
        [few] { $count } ملفات
        [many] { $count } ملفًا
       *[other] { $count } ملف
    }
seconds-left =
    { PLURAL($count) ->
        [zero] { $count } ثانية
        [one] ثانية واحدة
        [two] ثانيتان
        [few] { $count } ثوانٍ
        [many] { $count } ثانيةً
       *[other] { $count } ثانية
    }
minutes-left =
    { PLURAL($count) ->
        [zero] { $count } دقيقة
        [one] دقيقة واحدة
        [two] دقيقتان
        [few] { $count } دقائق
        [many] { $count } دقيقةً
       *[other] { $count } دقيقة
    }
hours-left =
    { PLURAL($count) ->
        [zero] { $count } ساعة
        [one] ساعة واحدة
        [two] ساعتان
        [few] { $count } ساعات
        [many] { $count } ساعةً
       *[other] { $count } ساعة
    }

## الوحدات ($value رقم منسَّق مسبقًا)

size-bytes = { $value } بايت
size-kib = { $value } كيلوبايت
size-mib = { $value } ميجابايت
size-gib = { $value } جيجابايت
speed-per-second = { $size }/ث
size-of-total = { $done } من { $total }

## حالات التحميل

status-waiting = في الانتظار
status-active = يعمل
status-paused = متوقف مؤقتًا
status-done = اكتمل
status-failed = فشل
status-open-browser = افتح المتصفح ليكمل
status-needs-ok = ينتظر موافقتك
status-scheduled = مجدول { $time }

## الشريط الجانبي

nav-all = كل التحميلات
nav-active = تعمل الآن
nav-done = مكتملة
nav-categories = الفئات
nav-queues = الطوابير
nav-settings = الإعدادات
queue-window = من { $start } إلى { $stop }

## الفئات المدمجة (المفتاح category-<id>)

category-programs = برامج
category-video = فيديوهات
category-music = صوتيات
category-documents = مستندات
category-archives = مضغوطات
category-none = بلا فئة

## خانة اللصق والأدوات

paste-placeholder = الصق رابطًا، أو اسحبه إلى هنا
paste-download = حمّل
action-resume = استكمال
action-pause = إيقاف مؤقت
action-delete = حذف
action-details = التفاصيل
action-open-folder = فتح المجلد
action-cancel-download = إلغاء التحميل
search-placeholder = بحث وتصفية

## الجدول

column-name = الاسم
column-size = الحجم
column-progress = التقدم
column-speed = السرعة
column-left = المتبقي
column-status = الحالة
progress-label =
    { PLURAL($count) ->
        [zero] { $percent }
        [one] { $percent } في جزء واحد
        [two] { $percent } في جزأين
        [few] { $percent } في { $count } أجزاء
        [many] { $percent } في { $count } جزءًا
       *[other] { $percent } في { $count } جزء
    }

## الشريط السفلي

footer-active =
    { PLURAL($count) ->
        [zero] لا شيء يعمل الآن
        [one] يعمل الآن: تحميل واحد
        [two] يعمل الآن: تحميلان
        [few] يعمل الآن: { $count } تحميلات
        [many] يعمل الآن: { $count } تحميلًا
       *[other] يعمل الآن: { $count } تحميل
    }
footer-total-speed = السرعة الكلية { $speed }
footer-limit-none = حد السرعة: بلا حد
footer-limit = حد السرعة: { $speed }
footer-waiting =
    { PLURAL($count) ->
        [zero] لا شيء في الانتظار
        [one] في الانتظار: تحميل واحد
        [two] في الانتظار: تحميلان
        [few] في الانتظار: { $count } تحميلات
        [many] في الانتظار: { $count } تحميلًا
       *[other] في الانتظار: { $count } تحميل
    }

## حوار الإضافة

add-title = إضافة تحميل
add-url = الرابط
add-probe-ranges = { $size }. السيرفر يدعم التقسيم، فيمكن التحميل بعدة اتصالات واستكماله لاحقًا.
add-probe-single = { $size }. السيرفر لا يدعم التقسيم، فسيُحمَّل باتصال واحد.
add-probe-unsure = { $size }. السيرفر يدعم التقسيم، لكنه لا يعطي الحجم أو ما يُتأكَّد به أن الملف لم يتغيّر، فسيُحمَّل باتصال واحد، ويبدأ من جديد إذا انقطع.
add-file-name = اسم الملف
add-category = الفئة
add-category-extension = { $category } (من الامتداد { $extension })
add-category-site = { $category } (من الموقع { $site })
add-queue = الطابور
add-folder = المجلد
add-change = تغيير
add-connections = الاتصالات
add-connections-hint = من { $min } إلى { $max }
add-limit = حد السرعة لهذا التحميل
add-limit-none = بلا حد
add-cancel = إلغاء
add-later = نزّل لاحقًا
add-start = ابدأ التحميل

## التفاصيل

details-title = تفاصيل التحميل
details-from-to = من { $site }، إلى { $folder }
details-file-map = خريطة الملف: كل مربع { $size }
details-map-done = وصل
details-map-writing = يُكتب الآن
details-map-split = موضع تقسيم
details-part = الجزء { $number }
details-part-done = انتهى
details-speed = السرعة
details-left = المتبقي
details-parts = الأجزاء
details-connections-value = يعمل { $active } من { $total }
details-resumable = يمكن استكماله
details-yes = نعم
details-no = لا
details-unknown = غير معروف
details-back = رجوع إلى القائمة
details-limit = حد السرعة
details-queue = الطابور
details-speed-chart = السرعة في آخر دقيقة
details-chart-then = قبل دقيقة
details-chart-now = الآن
details-failed = توقف التحميل: { $error }
details-open-browser = جاء هذا التحميل من المتصفح بكوكيزه، وزاجِل لا يحفظ الكوكيز. افتح المتصفح (ونفس الملف الشخصي) الذي جاء منه، فيأخذها زاجِل منه ويكمل وحده. وإن لم يعد ذلك المتصفح موجودًا، استكمل بلا كوكيز.
details-without-cookies = استكمل بلا كوكيز
details-needs-ok = هذا الملف قد يشغّل برامج على جهازك (برنامج أو مثبّت أو سكربت أو ملف Office بماكرو)، وجاء من صفحة في المتصفح، فلن يبدأ تحميله حتى تبدأه أنت. ابدأه فقط إن كنت تثق بالموقع.

## تحديث رابط التحميل

refresh-action = تحديث الرابط
refresh-title = تحديث رابط التحميل
refresh-hint = إذا انتهت صلاحية الرابط، فخذ من الموقع رابطًا جديدًا للملف نفسه والصقه هنا. يتأكد زاجِل أنه الملف نفسه، ثم يكمل من حيث توقف.
refresh-url = الرابط الجديد
refresh-cancel = إلغاء
refresh-go = حدّث واستكمل
refresh-not-stopped = أوقف التحميل مؤقتًا أولًا، ثم حدّث رابطه.
refresh-stopping = التحميل ما زال يتوقف. انتظر لحظة، ثم حاول مرة أخرى.
refresh-busy = تحديث رابط هذا التحميل جارٍ بالفعل.
refresh-cancelled = أُلغي تحديث الرابط، ولم يتغيّر التحميل.
refresh-done = اكتمل هذا التحميل، فلا يحتاج رابطًا جديدًا.
refresh-bad-link = زاجِل لا يقبل هذا الرابط: يجب أن يبدأ بـ http أو https، وألا يحمل اسم مستخدم أو كلمة مرور.
refresh-size = الرابط الجديد لملف آخر: حجمه { $theirs }، وحجم هذا التحميل { $ours }.
refresh-size-unknown = لا يُعرف حجم الملف الجديد أو حجم هذا التحميل، فلا يستطيع زاجِل التأكد أنه الملف نفسه.
refresh-version = الرابط الجديد لنسخة أخرى من الملف، فلن يكمل زاجِل منه حتى لا يفسد الملف.
refresh-validator-kind = السيرفر يصف الملف الجديد بطريقة مختلفة، فلا يستطيع زاجِل التأكد أنه الملف نفسه.
refresh-new-no-validator = سيرفر الرابط الجديد لا يعطي ما يتأكد به زاجِل أنه الملف نفسه.
refresh-no-validator = سيرفر هذا التحميل لم يعطِ ما يتأكد به زاجِل من الملف، فلا يمكن تحديث رابطه. ابدأ تحميلًا جديدًا بالرابط الجديد.
refresh-failed = لم يُحدَّث الرابط.

## الإعدادات

settings-title = الإعدادات
settings-applied = كل تغيير يُطبَّق فورًا.
settings-language = اللغة
settings-language-hint = تتغير الواجهة فورًا، دون إعادة تشغيل.
settings-theme = المظهر
settings-theme-hint = النظام يتبع إعداد الجهاز.
settings-theme-system = النظام
settings-theme-light = فاتح
settings-theme-dark = داكن
settings-digits = الأرقام
settings-digits-hint = في الأحجام والسرعات والأوقات.
settings-folder = مجلد التحميل
settings-folder-hint = وكل فئة يمكن أن يكون لها مجلدها.
settings-connections = الاتصالات لكل تحميل
settings-connections-hint = الافتراضي { $default }، والحد الأقصى { $max }.
settings-limit = حد السرعة للبرنامج كله
settings-limit-hint = يشمل كل التحميلات معًا.
settings-limit-none = بلا حد
settings-limit-set = حدد سرعة
settings-autostart = عند بدء النظام
settings-autostart-label = شغّل زاجِل مع بدء النظام
settings-notifications = الإشعارات
settings-notifications-label = أخبرني عندما يكتمل تحميل أو يفشل
settings-browser = المتصفح
settings-browser-hint = يسجّل زاجِل نفسه في Chrome وEdge وChromium وFirefox عند كل تشغيل، مثبَّتة كانت أو لا، لتصل إليه إضافته. وهذا الزر يعيد التسجيل ويقول النتيجة.
settings-browser-fix = أصلح التكامل مع المتصفح
settings-browser-ok = { $browser }: مسجَّل
settings-browser-failed = { $browser }: لم يُسجَّل ({ $error })
settings-browser-missing = لم يُعثر على zajil-host بجانب البرنامج، فلا يصل إليه أي متصفح.
settings-browser-temporary = يعمل البرنامج من مكان يزول حين يُغلق (AppImage، أو نقل ماك المؤقت)، فلم يُسجَّل في أي متصفح. ثبّت زاجِل في مكان دائم.
settings-browser-unsafe = يستطيع مستخدم آخر أن يغيّر zajil-host أو مجلده، فلم يُسجَّل في أي متصفح.
settings-browser-working = يسجّل…
settings-clipboard = الحافظة
settings-clipboard-label = راقب الحافظة، وضع رابط التحميل الذي أنسخه في خانة الإضافة
settings-clipboard-hint = ينظر زاجِل في الحافظة مرتين في الثانية بحثًا عن رابط ملف، ولا يحفظ ما فيها. ولا يتصل بالرابط ولا يضيف شيئًا قبل أن تضغط «تمام» في نافذة «إضافة رابط».
settings-export = صدّر الطوابير
settings-import = استورد طوابير
settings-export-warning = الملف فيه الروابط كما هي، وبعضها قد يحمل مفاتيح دخول مؤقتة. لا تنشره.

## الفئات

categories-title = الفئات
categories-hint = الفئة تُختار من موقع التحميل أولًا، ثم من امتداد الملف.
categories-new = فئة جديدة
categories-name = الاسم
categories-extensions = الامتدادات
categories-extensions-hint = اكتب امتدادًا واضغط Enter.
categories-add-extension = امتداد
categories-sites = المواقع
categories-sites-hint = كل ما يأتي من هذه المواقع يذهب إلى هنا.
categories-folder = المجلد
categories-folder-none = المجلد الذي يُعطى للتحميل
categories-try = جرّب
categories-try-hint = اكتب اسم ملف لترى فئته.
categories-delete = احذف الفئة

## شريط النظام

tray-open = افتح زاجِل
tray-pause-all = أوقف الكل
tray-resume-all = استكمل الكل
tray-quit = إنهاء زاجِل

## الإشعارات

notify-done-title = اكتمل التحميل
notify-done-body = { $name }
notify-failed-title = فشل التحميل
notify-failed-body = { $name }: { $reason }

## الإعدادات والفئات (مفاتيح كانت ناقصة: مراجعة #57)
settings-queues = الطوابير
settings-folder-choose = اختر مجلدًا
settings-folder-default = مجلد التحميلات في النظام
settings-import-keep-folders = استخدم المجلدات التي في الملف المستورد
settings-limit-value = الحد
settings-limit-unit = الوحدة
unit-kib-per-second = كيلوبايت/ث
unit-mib-per-second = ميجابايت/ث
export-done = صُدّرت الطوابير.
import-done =
    { PLURAL($count) ->
        [zero] لم يُضف أي تحميل.
        [one] أُضيف تحميل واحد.
        [two] أُضيف تحميلان.
        [few] أُضيفت { $count } تحميلات.
        [many] أُضيف { $count } تحميلًا.
       *[other] أُضيف { $count } تحميل.
    }
import-kept = هذه الطوابير أبقت إعداداتها: { $names }.
categories-add-site = أضف موقعًا
categories-folder-clear = بلا مجلد
categories-remove = احذف { $value }

## النافذة الرئيسية (M5.5، تصميم #83)

settings-edit-categories = حرّر الفئات
tb-add = إضافة رابط
tb-resume = استكمال
tb-pause = إيقاف
tb-pause-all = إيقاف الكل
tb-delete = حذف
tb-delete-done = حذف المكتمل
tb-settings = الإعدادات
tb-schedule = الجدولة
tb-run-queue = تشغيل الطابور
tb-halt-queue = إيقاف الطابور
tb-toolbar = أدوات التحميل
tree-unfinished = غير مكتملة
tree-finished = مكتملة
tree-label = التحميلات والفئات والطوابير
confirm-remove = احذف من القائمة
confirm-cancel = إلغاء
confirm-delete-done =
    { PLURAL($count) ->
        [zero] لا تحميلات مكتملة معروضة.
        [one] يُحذف من القائمة تحميل مكتمل واحد. الملف نفسه يبقى في مكانه.
        [two] يُحذف من القائمة تحميلان مكتملان. الملفان نفساهما يبقيان في مكانهما.
        [few] يُحذف من القائمة { $count } تحميلات مكتملة. الملفات نفسها تبقى في مكانها.
        [many] يُحذف من القائمة { $count } تحميلًا مكتملًا. الملفات نفسها تبقى في مكانها.
       *[other] يُحذف من القائمة { $count } تحميل مكتمل. الملفات نفسها تبقى في مكانها.
    }
confirm-delete =
    { PLURAL($count) ->
        [zero] لا شيء محدد.
        [one] يُحذف من القائمة تحميل واحد. ما نُزّل منه يبقى على القرص.
        [two] يُحذف من القائمة تحميلان. ما نُزّل منهما يبقى على القرص.
        [few] يُحذف من القائمة { $count } تحميلات. ما نُزّل منها يبقى على القرص.
        [many] يُحذف من القائمة { $count } تحميلًا. ما نُزّل منها يبقى على القرص.
       *[other] يُحذف من القائمة { $count } تحميل. ما نُزّل منها يبقى على القرص.
    }
tree-expand = أظهر ما تحتها
queue-main = الطابور الرئيسي
queue-starts = يبدأ { $time }
col-name = اسم الملف
col-size = الحجم
col-status = الحالة
col-left = الوقت المتبقي
col-speed = السرعة
col-last-try = آخر محاولة
col-description = الوصف
st-in-queue = في الطابور
st-paused = متوقف
st-error = خطأ { $code }
st-failed = فشل
footer-selected = التحديد: { $number }
addlink-title = إضافة رابط
addlink-url = الرابط
addlink-from-clipboard = أُخذ الرابط من الحافظة
addlink-ok = تمام
addlink-cancel = إلغاء

## M5.5: the menus, the right-click menu and the columns

menu-bar = القوائم
menu-file = ملف
menu-edit = تحرير
menu-downloads = التحميلات
menu-view = عرض
menu-help = مساعدة
cmd-add = إضافة رابط…
cmd-import = استيراد الطوابير…
cmd-export = تصدير الطوابير…
cmd-quit = خروج
cmd-select-all = تحديد الكل
cmd-find = بحث
cmd-copy-link = نسخ الرابط
cmd-delete-done = حذف المكتمل
cmd-resume = استكمال
cmd-pause = إيقاف
cmd-pause-all = إيقاف الكل
cmd-delete = حذف…
cmd-run-queue = تشغيل الطابور
cmd-halt-queue = إيقاف الطابور
cmd-schedule = الجدولة…
cmd-settings = الإعدادات…
cmd-categories = الفئات…
cmd-columns = الأعمدة
cmd-open = فتح
cmd-open-folder = فتح المجلد
cmd-progress = نافذة التقدّم
cmd-refresh-link = تحديث الرابط…
cmd-properties = الخصائص
cmd-about = عن زاجِل
cmd-move-to-queue = نقل إلى طابور
about-text = زاجِل، الإصدار { $version }
table-label = التحميلات
col-resize = عرض عمود { $column }
sys-hide = إخفاء { $app }
sys-hide-others = إخفاء الآخرين
sys-show-all = إظهار الكل
sys-undo = تراجع
sys-redo = إعادة
sys-cut = قص
sys-copy = نسخ
sys-paste = لصق
sys-window = نافذة
sys-minimize = تصغير
sys-zoom = تكبير
row-menu = قائمة التحميل
open-code-question = «{ $name }» برنامج أو ملف يمكنه تشغيل أوامر على جهازك. هل تفتحه؟
open-code-ok = فتح

## M5.5: the add windows
info-title = معلومات ملف التحميل
add-new-category = فئة جديدة
add-remember = تذكّر هذا المجلد لفئة «{ $category }»
add-resumable = يمكن استكماله
add-not-resumable = لا يمكن استكماله
add-later-queue = نزّل لاحقًا في طابور
add-checking = جارٍ سؤال السيرفر عن الملف…
add-about-file = عن الملف
addlink-info-open = نافذة «معلومات ملف التحميل» مفتوحة لرابط آخر. كمّله أو ألغِه أولًا.
add-remember-failed = أُضيف التحميل، لكن لم يُحفظ المجلد للفئة: { $error }

## Quit with downloads running (owner's decision on #83)
quit-question =
    { PLURAL($count) ->
        [zero] لا تحميلات تعمل الآن. هل تُنهي زاجِل؟
        [one] يعمل الآن تحميل واحد. هل تُنهي زاجِل؟
        [two] يعمل الآن تحميلان. هل تُنهي زاجِل؟
        [few] يعمل الآن { $count } تحميلات. هل تُنهي زاجِل؟
        [many] يعمل الآن { $count } تحميلًا. هل تُنهي زاجِل؟
       *[other] يعمل الآن { $count } تحميل. هل تُنهي زاجِل؟
    }
quit-ok = إنهاء زاجِل

## الفيديو (M6)

video-soon = تحميل الفيديو قادم قريبًا.

## The main window's title bar (the design of #83)
app-title = زاجِل لإدارة التحميل
win-minimize = تصغير
win-maximize = تكبير
win-restore = استعادة
win-close = إغلاق

## Closing the window (owner's decision, after #105)
tray-still-running = زاجِل ما زال يعمل في شريط النظام، والتحميلات تكمل. اختر «إنهاء زاجِل» من أيقونته لإنهائه.
settings-closing = إغلاق النافذة
settings-closing-label = إغلاق النافذة يُنهي زاجِل
settings-closing-hint = دون هذا الخيار تختفي النافذة ويبقى زاجِل في شريط النظام والتحميلات تكمل، كما في IDM.
settings-closing-no-tray = لا يظهر شريط النظام في سطح المكتب هذا، فإغلاق النافذة يُنهي زاجِل دائمًا (ويسأل أولًا إن كانت هناك تحميلات تعمل).

## M5.5: the progress window (design window 4)
progress-title = تقدّم التحميل
progress-tab-status = حالة التحميل
progress-tab-limit = حد السرعة
progress-tab-finish = عند الانتهاء
progress-status = الحالة
progress-size = حجم الملف
progress-done = ما نُزّل
progress-resume = الاستكمال
progress-resume-yes = ممكن
progress-resume-no = غير ممكن
progress-receiving = يستقبل البيانات…
progress-more = إظهار التفاصيل
progress-less = إخفاء التفاصيل
progress-close = إغلاق
progress-connection = الاتصال
progress-got = ما نزّله
progress-working = يعمل { $number }
progress-left-part = باقٍ
progress-no-parts = تظهر الأجزاء حين يبدأ التحميل.
progress-limit-on = حدّد سرعة هذا التحميل
progress-limit-unit = كيلوبايت/ث
progress-limit-hint = حد البرنامج كله في الإعدادات يبقى ساريًا كذلك.
progress-limit-save = حفظ
progress-limit-saved = حُفظ الحد.
progress-limit-next = حُفظ الحد، ويسري حين يُستكمل التحميل في المرة القادمة.
progress-finish-hint = ما تفعله هذه النافذة حين ينتهي التحميل وهي مفتوحة:
progress-finish-open = افتح الملف
progress-finish-folder = افتح المجلد
progress-finish-close = أغلق هذه النافذة
progress-gone = أُزيل هذا التحميل من القائمة.
progress-parts-done = اكتمل التحميل، وكل أجزائه وصلت.
progress-limit-bad = اكتب رقمًا صحيحًا من { $min } إلى { $max }.
progress-connections = { $connections }، { $working }
