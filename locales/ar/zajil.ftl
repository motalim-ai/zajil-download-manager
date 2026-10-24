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
        [zero] لا قوائم
        [one] قائمة واحدة
        [two] قائمتان
        [few] { $count } قوائم
        [many] { $count } قائمةً
       *[other] { $count } قائمة
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
status-scheduled = مجدول { $time }

## الشريط الجانبي

nav-all = كل التحميلات
nav-active = تعمل الآن
nav-done = مكتملة
nav-categories = الفئات
nav-queues = القوائم
nav-settings = الإعدادات
queue-window = من { $start } إلى { $stop }

## الفئات المدمجة (المفتاح category-<id>)

category-programs = برامج
category-video = أفلام
category-music = موسيقى
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
search-placeholder = ابحث في التحميلات

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
add-file-name = اسم الملف
add-category = الفئة
add-category-auto = { $category } (من الامتداد { $extension })
add-queue = القائمة
add-folder = المجلد
add-change = تغيير
add-connections = الاتصالات
add-connections-hint = من { $min } إلى { $max }
add-limit = حد السرعة لهذا التحميل
add-limit-none = بلا حد
add-cancel = إلغاء
add-later = أضف إلى القائمة
add-start = ابدأ التحميل

## التفاصيل

details-title = تفاصيل التحميل
details-from-to = من { $site }، إلى { $folder }
details-file-map = خريطة الملف: كل مربع { $size }
details-map-done = وصل
details-map-writing = يُكتب الآن
details-map-split = موضع تقسيم
details-connection = الاتصال { $number }
details-connection-done = انتهى
details-speed = السرعة
details-left = المتبقي
details-connections = الاتصالات
details-connections-value = يعمل { $active } من { $total }
details-resumable = يمكن استكماله
details-yes = نعم
details-no = لا
details-limit = حد السرعة
details-queue = القائمة
details-speed-chart = السرعة في آخر دقيقة
details-chart-then = قبل دقيقة
details-chart-now = الآن

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
settings-export = صدّر القوائم
settings-import = استورد قوائم
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
