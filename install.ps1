# PrinterTUI installer for Windows 10/11.
#   irm https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.ps1 | iex
# Asks for a language, then Install the first time, Update or Uninstall afterwards, and offers
# LibreOffice and a desktop shortcut. Installs for this user only (no administrator), on the PATH and in the Start menu.
# $env:PRINTERTUI_PREVIEW = 'fresh' | 'installed' | 'jam' | 'smudge' plays the menus and animations
# without changing anything (see test/installer-preview.ps1).
# Runs through `iex` in the user's own PowerShell: never `exit`, it would close their window.
# Everything runs inside `& { }`, so its settings ($ErrorActionPreference...), functions and
# variables stay in there instead of changing the user's session; the state the functions share
# lives in $S rather than in script-scope variables, which `iex` would leave in the session.

& {
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# process-wide, so put back at the end (a host without a console, like CI, has no encoding to set)
$oldProtocol = [Net.ServicePointManager]::SecurityProtocol
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$oldEncoding = $null
try { $oldEncoding = [Console]::OutputEncoding; [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$repo = 's0artak/PrinterTUI'
$preview = $env:PRINTERTUI_PREVIEW
$E = [char]27
$base = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { [IO.Path]::GetTempPath() }
# where the app keeps its settings (the preview runs where there is no APPDATA too)
$roaming = if ($env:APPDATA) { $env:APPDATA } else { $base }
$configFile = Join-Path $roaming 'printertui\config'
$dir = Join-Path $base 'Programs\PrinterTUI'
$exe = Join-Path $dir 'printertui.exe'
$startMenu = [Environment]::GetFolderPath('Programs')
$shortcut = if ($startMenu) { Join-Path $startMenu 'PrinterTUI.lnk' } else { 'PrinterTUI.lnk' }
$desktop = [Environment]::GetFolderPath('Desktop')
$desktopLink = if ($desktop) { Join-Path $desktop 'PrinterTUI.lnk' } else { 'Desktop\PrinterTUI.lnk' }
# menus and animations only when a person is watching
$tty = -not [Console]::IsOutputRedirected -and -not [Console]::IsInputRedirected

# Windows PowerShell 5.1 in the old console needs escape sequences switched on
if ($tty -and $PSVersionTable.PSVersion.Major -lt 6 -and -not $Host.UI.SupportsVirtualTerminal) {
    Add-Type -Namespace PrinterTUI -Name Console -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int h);
[DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr h, out int m);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(IntPtr h, int m);
'@
    $h = [PrinterTUI.Console]::GetStdHandle(-11)
    $m = 0
    [void][PrinterTUI.Console]::GetConsoleMode($h, [ref]$m)
    [void][PrinterTUI.Console]::SetConsoleMode($h, $m -bor 4)
}

function W([string]$s) { [Console]::Write($s) }
function Say([string]$t) { W "  $E[38;5;69m▸$E[0m $t`n" }
# The printer's speech bubble, as in the app. With -Pet it goes beside the printer just drawn,
# its tail pointing at it; -Color 203 draws it red, for errors.
function Talk([string]$t, [switch]$Pet, [int]$Color = 69) {
    if (-not $tty) { W "`n  $t`n`n"; return }
    $cols = try { [Console]::WindowWidth } catch { 80 }
    $left = if ($Pet -and $S.drawn -and $cols -ge 64) { 24 } else { 2 }
    $lines = Wrap $t ([math]::Min(60, $cols - $left - 5))
    $w = ($lines | Measure-Object -Property Cols -Maximum).Maximum
    $c = "$E[38;5;$($Color)m"
    $go = "$E[$($left + 1)G"
    $bar = [string]::new([char]0x2500, $w + 2)
    # beside the printer: back up to its second row, so the tail meets its lights
    if ($left -gt 2) { W "$E[$($S.drawn - 1)A" } else { W "`n" }
    W "$go$c$([char]0x256D)$bar$([char]0x256E)$E[0m`n"
    $first = $true
    foreach ($l in $lines) {
        $edge = if ($left -gt 2 -and $first) { [char]0x25C0 } else { [char]0x2502 }
        W "$go$c$edge$E[0m $($l.Text)$(' ' * ($w - $l.Cols)) $c$([char]0x2502)$E[0m`n"
        $first = $false
    }
    W "$go$c$([char]0x2570)$bar$([char]0x256F)$E[0m`n"
    if ($left -gt 2 -and $lines.Count + 3 -lt $S.drawn) { W "$E[$($S.drawn - $lines.Count - 3)B" }
    W "`n"
    $S.drawn = 0
}

# --- sounds: the app's (see src/sound.rs), synthesized here, as there are no files to ship ---
# The app's volume when it has one (an update keeps it, muted too), else its gentle default.
$volume = 50
if (Test-Path $configFile) {
    $found = [IO.File]::ReadAllLines($configFile) | Select-String '^volume=(\d+)$' | Select-Object -Last 1
    if ($found) { $volume = [int]$found.Matches[0].Groups[1].Value }
}
# what the functions share: rows of art drawn, sounds made and playing, the paper, the frame
$S = @{ drawn = 0; sounds = @{}; playing = [System.Collections.ArrayList]::new(); paper = $null; n = 0 }

# Plays a sound (boot blip print done jam bye) in the background, when a person is listening.
function Play([string]$name) {
    if (-not $tty -or $volume -le 0) { return }
    try {
        if (-not $S.sounds[$name]) { $S.sounds[$name] = Synth $name $volume }
        $player = [System.Media.SoundPlayer]::new([IO.MemoryStream]::new($S.sounds[$name]))
        $player.Play()
        # kept alive while it plays
        [void]$S.playing.Add($player)
    } catch { }
}

# A sound at a volume as the bytes of an 8-bit WAV file.
function Synth([string]$name, [int]$vol) {
    $R = 11025
    $tau = 2 * [math]::PI
    $s = [System.Collections.Generic.List[double]]::new()
    $rnd = [Random]::new(7)
    function Note($f, $secs, $amp) {
        for ($i = 0; $i -lt [int]($secs * $R); $i++) {
            $t = $i / $R
            $shape = [math]::Min($t * 400, 1) * [math]::Exp(-$t * 9)
            $s.Add($amp * $shape * ([math]::Sin($tau * $f * $t) + 0.3 * [math]::Sin(3 * $tau * $f * $t)) / 1.3)
        }
    }
    switch ($name) {
        'boot' { Note 523.25 0.12 0.56; Note 659.25 0.12 0.56; Note 783.99 0.12 0.56 }
        'bye' { Note 783.99 0.14 0.56; Note 659.25 0.14 0.56; Note 523.25 0.2 0.56 }
        'blip' { Note 1318.5 0.05 0.35 }
        'done' { Note 880 0.12 0.7; Note 1318.5 0.3 0.7 }
        'print' {
            for ($i = 0; $i -lt [int](1.1 * $R); $i++) {
                $t = $i / $R
                $fade = [math]::Min($t * 20, 1) * [math]::Min((1.1 - $t) * 20, 1)
                $x = $t * 70 + 0.5 * [math]::Sin($t * 3)
                $saw = 2 * ($x - [math]::Floor($x)) - 1
                $s.Add(0.55 * $fade * (0.55 + 0.45 * [math]::Sin($tau * 7 * $t)) * (0.7 * $saw + 0.3 * ($rnd.NextDouble() * 2 - 1)))
            }
            $last = 0
            $n = [int](0.18 * $R)
            for ($i = 0; $i -lt $n; $i++) {
                $x = $rnd.NextDouble() * 2 - 1
                $s.Add(0.12 * [math]::Pow(1 - $i / $n, 2) * ($x - $last))
                $last = $x
            }
        }
        'jam' {
            for ($k = 0; $k -lt 3; $k++) {
                for ($i = 0; $i -lt [int](0.16 * $R); $i++) {
                    $t = $i / $R
                    $shape = [math]::Min($t * 200, 1) * (1 - $t / 0.16)
                    $square = if ($t * 95 - [math]::Floor($t * 95) -lt 0.5) { 1 } else { -1 }
                    $s.Add(0.42 * $shape * (0.8 * $square + 0.2 * ($rnd.NextDouble() * 2 - 1)))
                }
                for ($i = 0; $i -lt [int](0.05 * $R); $i++) { $s.Add(0) }
            }
            $ph = 0
            for ($i = 0; $i -lt [int](0.35 * $R); $i++) {
                $t = $i / $R
                $ph += $tau * (300 - 500 * $t) / $R
                $s.Add(0.5 * (1 - $t / 0.35) * [math]::Sin($ph))
            }
        }
    }
    $gain = [math]::Pow($vol / 100, 2)
    $out = [IO.MemoryStream]::new()
    $w = [IO.BinaryWriter]::new($out)
    $ascii = [Text.Encoding]::ASCII
    $w.Write($ascii.GetBytes('RIFF')); $w.Write([int](36 + $s.Count)); $w.Write($ascii.GetBytes('WAVEfmt '))
    $w.Write([int]16); $w.Write([int16]1); $w.Write([int16]1); $w.Write([int]$R); $w.Write([int]$R); $w.Write([int16]1); $w.Write([int16]8)
    $w.Write($ascii.GetBytes('data')); $w.Write([int]$s.Count)
    foreach ($v in $s) { $w.Write([byte][math]::Max(0, [math]::Min(255, [math]::Round(128 + 127 * $v * $gain)))) }
    $w.Flush()
    , $out.ToArray()
}

# Screen columns of a text: Chinese takes two, the marks Hindi, Bengali and Arabic draw over the
# letter before take none.
function Cols([string]$s) {
    $w = 0
    foreach ($ch in $s.ToCharArray()) {
        $u = [int]$ch
        if ([char]::IsLowSurrogate($ch)) { continue }
        if ([char]::IsHighSurrogate($ch)) { $w += 2; continue }
        $cat = [Globalization.CharUnicodeInfo]::GetUnicodeCategory($ch)
        if ($cat -in 'NonSpacingMark', 'EnclosingMark', 'Format') { continue }
        $wide = ($u -ge 0x1100 -and $u -le 0x115F) -or ($u -ge 0x2E80 -and $u -le 0xA4CF) -or ($u -ge 0xAC00 -and $u -le 0xD7A3) -or
            ($u -ge 0xF900 -and $u -le 0xFAFF) -or ($u -ge 0xFE30 -and $u -le 0xFE4F) -or ($u -ge 0xFF00 -and $u -le 0xFF60) -or ($u -ge 0xFFE0 -and $u -le 0xFFE6)
        $w += if ($wide) { 2 } else { 1 }
    }
    $w
}

# Word-wraps a text into lines of at most $max screen columns.
function Wrap([string]$t, [int]$max) {
    $out = @()
    $line = ''
    $lw = 0
    foreach ($word in $t -split ' ') {
        $ww = Cols $word
        if ($lw -gt 0 -and $lw + 1 + $ww -gt $max) {
            $out += [pscustomobject]@{ Cols = $lw; Text = $line }
            $line = $word
            $lw = $ww
        } elseif ($lw -gt 0) {
            $line += " $word"
            $lw += 1 + $ww
        } else {
            $line = $word
            $lw = $ww
        }
    }
    $out += [pscustomobject]@{ Cols = $lw; Text = $line }
    , $out
}
function Fail([string]$t) { W "  $E[31m✗ $t$E[0m`n" }
# every step that changes the system goes through Step, so the preview can skip it
function Step([string]$what, [scriptblock]$do) {
    if ($preview) {
        W "    $E[2m$($T.skip) $what$E[0m`n"
        Start-Sleep -Milliseconds 300
    } else {
        & $do
    }
}

# --- languages: the 10 most spoken (Ethnologue), texts shared with install.sh --------------------
$Langs = [ordered]@{
    en = @{
        name = '🇬🇧 English'
        hint = '↑↓ move · enter pick · q quit'
        prev = '(preview: nothing is installed or removed)'
        skip = 'preview, skipped:'
        hi_new = 'Hi! Looks like we haven''t met. Warm up the printer?'
        hi_old = 'Oh, you already have me. Fresh copy, or shall I pack up?'
        install = 'Install'
        update = 'Update'
        uninstall = 'Uninstall'
        download = 'Downloading PrinterTUI'
        jam = 'Paper jam! The download didn''t come through.'
        smudge = 'Smudged page! Something''s off with that download.'
        e_net = 'check your internet connection and run the installer again'
        e_sum = 'the download doesn''t match its checksum, so nothing was installed'
        e_winget = 'winget is not on this Windows, so LibreOffice was skipped: get it from libreoffice.org'
        put = 'Putting printertui in'
        done = 'Page printed, ink dry, ready to go. Run: printertui'
        rm_cmd = 'Removing the printertui command'
        rm_cfg = 'Removing settings and temporary files'
        bye = 'Paper''s back in the tray. Bye for now!'
        quit = 'No worries, nothing was touched.'
        extras = 'Want some extras? All optional.'
        check_hint = '↑↓ move · space mark · enter continue · q quit'
        lo = 'LibreOffice: print Word, Excel and other non-PDF files'
        extras_inst = 'Installing the extras'
        left_win = 'Your printers were left as they are.'
        start = 'Also in the Start menu: PrinterTUI'
        desktop = 'Desktop shortcut for PrinterTUI'
    }
    zh = @{
        name = '🇨🇳 中文'
        hint = '↑↓ 移动 · 回车 选择 · q 退出'
        prev = '（预览：不会安装或删除任何东西）'
        skip = '预览，已跳过：'
        hi_new = '你好！我们好像还没见过面。要预热打印机吗？'
        hi_old = '哦，我已经在这儿了。重新装一份，还是我收拾东西走人？'
        install = '安装'
        update = '更新'
        uninstall = '卸载'
        download = '正在下载 PrinterTUI'
        jam = '卡纸了！下载没有成功。'
        smudge = '页面印花了！这次下载有问题。'
        e_net = '请检查网络连接后重新运行安装程序'
        e_sum = '下载的文件与校验和不符，什么都没有安装'
        e_winget = '此 Windows 没有 winget，已跳过 LibreOffice：请从 libreoffice.org 下载'
        put = '正在把 printertui 放到'
        done = '页面已打印，墨迹已干，一切就绪。运行：printertui'
        rm_cmd = '正在删除 printertui 命令'
        rm_cfg = '正在删除设置和临时文件'
        bye = '纸已放回纸盒。再见！'
        quit = '没关系，什么都没改动。'
        extras = '要来点附加组件吗？全部可选。'
        check_hint = '↑↓ 移动 · 空格 勾选 · 回车 继续 · q 退出'
        lo = 'LibreOffice：打印 Word、Excel 等非 PDF 文件'
        extras_inst = '正在安装附加组件'
        left_win = '你的打印机保持原样。'
        start = '开始菜单里也有：PrinterTUI'
        desktop = '在桌面创建 PrinterTUI 快捷方式'
    }
    hi = @{
        name = '🇮🇳 हिन्दी'
        hint = '↑↓ चुनें · enter पक्का करें · q बाहर'
        prev = '(पूर्वावलोकन: कुछ भी इंस्टॉल या हटाया नहीं जाता)'
        skip = 'पूर्वावलोकन, छोड़ा गया:'
        hi_new = 'नमस्ते! लगता है हम पहली बार मिल रहे हैं। प्रिंटर गरम करें?'
        hi_old = 'अरे, मैं तो पहले से यहाँ हूँ। नई कॉपी, या मैं अपना सामान बाँधूँ?'
        install = 'इंस्टॉल करें'
        update = 'अपडेट करें'
        uninstall = 'अनइंस्टॉल करें'
        download = 'PrinterTUI डाउनलोड हो रहा है'
        jam = 'पेपर जाम! डाउनलोड नहीं हो पाया।'
        smudge = 'पेज पर धब्बा! इस डाउनलोड में कुछ गड़बड़ है।'
        e_net = 'इंटरनेट कनेक्शन जाँचें और इंस्टॉलर फिर से चलाएँ'
        e_sum = 'डाउनलोड चेकसम से मेल नहीं खाता, इसलिए कुछ भी इंस्टॉल नहीं हुआ'
        e_winget = 'इस Windows में winget नहीं है, इसलिए LibreOffice छोड़ दिया: libreoffice.org से लें'
        put = 'printertui को यहाँ रखा जा रहा है:'
        done = 'पेज छप गया, स्याही सूख गई, सब तैयार। चलाएँ: printertui'
        rm_cmd = 'printertui कमांड हटाई जा रही है'
        rm_cfg = 'सेटिंग्स और अस्थायी फ़ाइलें हटाई जा रही हैं'
        bye = 'कागज़ वापस ट्रे में। फिर मिलेंगे!'
        quit = 'कोई बात नहीं, कुछ भी नहीं बदला।'
        extras = 'कुछ अतिरिक्त चाहिए? सब वैकल्पिक हैं।'
        check_hint = '↑↓ चुनें · space निशान · enter आगे · q बाहर'
        lo = 'LibreOffice: Word, Excel और दूसरी गैर-PDF फ़ाइलें छापें'
        extras_inst = 'अतिरिक्त चीज़ें इंस्टॉल हो रही हैं'
        left_win = 'आपके प्रिंटर जैसे थे वैसे ही हैं।'
        start = 'स्टार्ट मेनू में भी: PrinterTUI'
        desktop = 'डेस्कटॉप पर PrinterTUI का शॉर्टकट'
    }
    es = @{
        name = '🇪🇸 Español'
        hint = '↑↓ mover · enter elegir · q salir'
        prev = '(vista previa: no se instala ni se borra nada)'
        skip = 'vista previa, omitido:'
        hi_new = '¡Hola! Parece que no nos conocemos. ¿Calentamos la impresora?'
        hi_old = 'Anda, ya me tienes instalada. ¿Copia nueva o recojo mis cosas?'
        install = 'Instalar'
        update = 'Actualizar'
        uninstall = 'Desinstalar'
        download = 'Descargando PrinterTUI'
        jam = '¡Atasco de papel! La descarga no ha llegado.'
        smudge = '¡Página emborronada! Algo raro pasa con esa descarga.'
        e_net = 'revisa tu conexión a internet y vuelve a ejecutar el instalador'
        e_sum = 'la descarga no coincide con su checksum, así que no he instalado nada'
        e_winget = 'este Windows no tiene winget, así que me salto LibreOffice: descárgalo de libreoffice.org'
        put = 'Poniendo printertui en'
        done = 'Página impresa, tinta seca, todo listo. Ejecuta: printertui'
        rm_cmd = 'Quitando el comando printertui'
        rm_cfg = 'Borrando ajustes y archivos temporales'
        bye = 'El papel ha vuelto a la bandeja. ¡Hasta pronto!'
        quit = 'Tranquilo, no he tocado nada.'
        extras = '¿Unos extras? Todos son opcionales.'
        check_hint = '↑↓ mover · espacio marcar · enter seguir · q salir'
        lo = 'LibreOffice: imprimir Word, Excel y otros archivos que no son PDF'
        extras_inst = 'Instalando los extras'
        left_win = 'Tus impresoras se quedan como estaban.'
        start = 'También en el menú Inicio: PrinterTUI'
        desktop = 'Acceso directo a PrinterTUI en el escritorio'
    }
    ar = @{
        name = '🇸🇦 العربية'
        hint = '↑↓ تنقّل · enter اختيار · q خروج'
        prev = '(معاينة: لا يُثبَّت ولا يُحذف شيء)'
        skip = 'معاينة، تم التخطي:'
        hi_new = 'أهلاً! يبدو أننا لم نلتقِ من قبل. نسخّن الطابعة؟'
        hi_old = 'أوه، أنا مثبّت بالفعل. نسخة جديدة، أم أحزم أغراضي؟'
        install = 'تثبيت'
        update = 'تحديث'
        uninstall = 'إزالة'
        download = 'جارٍ تنزيل PrinterTUI'
        jam = 'انحشار الورق! لم يكتمل التنزيل.'
        smudge = 'صفحة ملطّخة! هناك خطب ما في هذا التنزيل.'
        e_net = 'تحقّق من اتصالك بالإنترنت ثم أعد تشغيل المثبّت'
        e_sum = 'الملف المنزّل لا يطابق المجموع الاختباري، لذلك لم يُثبَّت شيء'
        e_winget = 'لا يوجد winget في Windows هذا، لذا تخطيت LibreOffice: نزّله من libreoffice.org'
        put = 'جارٍ وضع printertui في'
        done = 'طُبعت الصفحة وجفّ الحبر، كل شيء جاهز. شغّل: printertui'
        rm_cmd = 'جارٍ حذف أمر printertui'
        rm_cfg = 'جارٍ حذف الإعدادات والملفات المؤقتة'
        bye = 'عاد الورق إلى الدرج. إلى اللقاء!'
        quit = 'لا بأس، لم يتغير شيء.'
        extras = 'هل تريد بعض الإضافات؟ كلها اختيارية.'
        check_hint = '↑↓ تنقّل · space تحديد · enter متابعة · q خروج'
        lo = 'LibreOffice: طباعة ملفات Word و Excel وغيرها من غير PDF'
        extras_inst = 'جارٍ تثبيت الإضافات'
        left_win = 'بقيت طابعاتك كما هي.'
        start = 'موجود أيضاً في قائمة ابدأ: PrinterTUI'
        desktop = 'اختصار PrinterTUI على سطح المكتب'
    }
    fr = @{
        name = '🇫🇷 Français'
        hint = '↑↓ bouger · entrée choisir · q quitter'
        prev = '(aperçu : rien n''est installé ni supprimé)'
        skip = 'aperçu, ignoré :'
        hi_new = 'Salut ! On ne se connaît pas encore. On fait chauffer l''imprimante ?'
        hi_old = 'Oh, je suis déjà là. Une copie toute neuve, ou je fais mes valises ?'
        install = 'Installer'
        update = 'Mettre à jour'
        uninstall = 'Désinstaller'
        download = 'Téléchargement de PrinterTUI'
        jam = 'Bourrage papier ! Le téléchargement n''a pas abouti.'
        smudge = 'Page tachée ! Ce téléchargement a un souci.'
        e_net = 'vérifiez votre connexion internet puis relancez l''installateur'
        e_sum = 'le téléchargement ne correspond pas à sa somme de contrôle, rien n''a été installé'
        e_winget = 'ce Windows n''a pas winget, LibreOffice est donc ignoré : téléchargez-le sur libreoffice.org'
        put = 'Installation de printertui dans'
        done = 'Page imprimée, encre sèche, tout est prêt. Lancez : printertui'
        rm_cmd = 'Suppression de la commande printertui'
        rm_cfg = 'Suppression des réglages et fichiers temporaires'
        bye = 'Le papier est retourné dans le bac. À bientôt !'
        quit = 'Pas de souci, rien n''a été modifié.'
        extras = 'Quelques extras ? Tous optionnels.'
        check_hint = '↑↓ bouger · espace cocher · entrée continuer · q quitter'
        lo = 'LibreOffice : imprimer Word, Excel et autres fichiers non PDF'
        extras_inst = 'Installation des extras'
        left_win = 'Vos imprimantes restent telles quelles.'
        start = 'Aussi dans le menu Démarrer : PrinterTUI'
        desktop = 'Raccourci PrinterTUI sur le bureau'
    }
    bn = @{
        name = '🇧🇩 বাংলা'
        hint = '↑↓ সরান · enter বাছুন · q বের হন'
        prev = '(প্রিভিউ: কিছুই ইনস্টল বা মোছা হয় না)'
        skip = 'প্রিভিউ, বাদ দেওয়া হয়েছে:'
        hi_new = 'হ্যালো! মনে হচ্ছে আমাদের আগে দেখা হয়নি। প্রিন্টারটা গরম করব?'
        hi_old = 'আরে, আমি তো আগেই আছি। নতুন কপি, নাকি আমি গুছিয়ে চলে যাব?'
        install = 'ইনস্টল'
        update = 'আপডেট'
        uninstall = 'আনইনস্টল'
        download = 'PrinterTUI ডাউনলোড হচ্ছে'
        jam = 'কাগজ আটকে গেছে! ডাউনলোড হয়নি।'
        smudge = 'পাতায় দাগ! এই ডাউনলোডে কিছু গোলমাল আছে।'
        e_net = 'ইন্টারনেট সংযোগ দেখে নিন, তারপর ইনস্টলার আবার চালান'
        e_sum = 'ডাউনলোড চেকসামের সাথে মেলেনি, তাই কিছুই ইনস্টল হয়নি'
        e_winget = 'এই Windows-এ winget নেই, তাই LibreOffice বাদ দিলাম: libreoffice.org থেকে নিন'
        put = 'printertui রাখা হচ্ছে:'
        done = 'পাতা ছাপা হয়েছে, কালি শুকিয়েছে, সব তৈরি। চালান: printertui'
        rm_cmd = 'printertui কমান্ড সরানো হচ্ছে'
        rm_cfg = 'সেটিংস আর অস্থায়ী ফাইল মোছা হচ্ছে'
        bye = 'কাগজ আবার ট্রেতে। আবার দেখা হবে!'
        quit = 'চিন্তা নেই, কিছুই বদলানো হয়নি।'
        extras = 'কিছু বাড়তি জিনিস চান? সবই ঐচ্ছিক।'
        check_hint = '↑↓ সরান · space চিহ্ন · enter এগোন · q বের হন'
        lo = 'LibreOffice: Word, Excel আর অন্য PDF-নয় এমন ফাইল ছাপুন'
        extras_inst = 'বাড়তি জিনিস ইনস্টল হচ্ছে'
        left_win = 'আপনার প্রিন্টার যেমন ছিল তেমনই আছে।'
        start = 'স্টার্ট মেনুতেও আছে: PrinterTUI'
        desktop = 'ডেস্কটপে PrinterTUI-এর শর্টকাট'
    }
    pt = @{
        name = '🇧🇷 Português'
        hint = '↑↓ mover · enter escolher · q sair'
        prev = '(prévia: nada é instalado nem removido)'
        skip = 'prévia, ignorado:'
        hi_new = 'Oi! Parece que ainda não nos conhecemos. Vamos esquentar a impressora?'
        hi_old = 'Ah, eu já estou aqui. Cópia nova, ou arrumo as malas?'
        install = 'Instalar'
        update = 'Atualizar'
        uninstall = 'Desinstalar'
        download = 'Baixando o PrinterTUI'
        jam = 'Papel atolado! O download não chegou.'
        smudge = 'Página borrada! Tem algo errado com esse download.'
        e_net = 'verifique sua conexão com a internet e rode o instalador de novo'
        e_sum = 'o download não bate com o checksum, então nada foi instalado'
        e_winget = 'este Windows não tem winget, então pulei o LibreOffice: baixe em libreoffice.org'
        put = 'Colocando o printertui em'
        done = 'Página impressa, tinta seca, tudo pronto. Execute: printertui'
        rm_cmd = 'Removendo o comando printertui'
        rm_cfg = 'Removendo configurações e arquivos temporários'
        bye = 'O papel voltou para a bandeja. Até mais!'
        quit = 'Tudo bem, nada foi alterado.'
        extras = 'Quer uns extras? Todos opcionais.'
        check_hint = '↑↓ mover · espaço marcar · enter continuar · q sair'
        lo = 'LibreOffice: imprimir Word, Excel e outros arquivos que não são PDF'
        extras_inst = 'Instalando os extras'
        left_win = 'Suas impressoras ficaram como estavam.'
        start = 'Também no menu Iniciar: PrinterTUI'
        desktop = 'Atalho do PrinterTUI na área de trabalho'
    }
    ru = @{
        name = '🇷🇺 Русский'
        hint = '↑↓ выбор · enter ок · q выход'
        prev = '(предпросмотр: ничего не устанавливается и не удаляется)'
        skip = 'предпросмотр, пропущено:'
        hi_new = 'Привет! Кажется, мы ещё не знакомы. Разогреть принтер?'
        hi_old = 'О, я уже установлен. Свежая копия или мне собирать вещи?'
        install = 'Установить'
        update = 'Обновить'
        uninstall = 'Удалить'
        download = 'Скачиваю PrinterTUI'
        jam = 'Бумагу зажевало! Загрузка не удалась.'
        smudge = 'Страница смазана! С этой загрузкой что-то не так.'
        e_net = 'проверьте подключение к интернету и запустите установщик снова'
        e_sum = 'загрузка не совпадает с контрольной суммой, поэтому ничего не установлено'
        e_winget = 'в этом Windows нет winget, поэтому LibreOffice пропущен: скачайте его с libreoffice.org'
        put = 'Кладу printertui в'
        done = 'Страница напечатана, чернила высохли, всё готово. Запуск: printertui'
        rm_cmd = 'Удаляю команду printertui'
        rm_cfg = 'Удаляю настройки и временные файлы'
        bye = 'Бумага вернулась в лоток. Пока!'
        quit = 'Ничего страшного, ничего не изменено.'
        extras = 'Немного дополнений? Всё по желанию.'
        check_hint = '↑↓ выбор · пробел отметить · enter дальше · q выход'
        lo = 'LibreOffice: печать Word, Excel и других файлов не в PDF'
        extras_inst = 'Устанавливаю дополнения'
        left_win = 'Ваши принтеры остались как были.'
        start = 'Также в меню «Пуск»: PrinterTUI'
        desktop = 'Ярлык PrinterTUI на рабочем столе'
    }
    id = @{
        name = '🇮🇩 Bahasa Indonesia'
        hint = '↑↓ pindah · enter pilih · q keluar'
        prev = '(pratinjau: tidak ada yang dipasang atau dihapus)'
        skip = 'pratinjau, dilewati:'
        hi_new = 'Halo! Sepertinya kita belum kenal. Panaskan printernya?'
        hi_old = 'Oh, aku sudah terpasang. Salinan baru, atau aku berkemas?'
        install = 'Pasang'
        update = 'Perbarui'
        uninstall = 'Copot'
        download = 'Mengunduh PrinterTUI'
        jam = 'Kertas macet! Unduhannya gagal.'
        smudge = 'Halamannya belepotan! Ada yang aneh dengan unduhan itu.'
        e_net = 'periksa koneksi internet lalu jalankan installer lagi'
        e_sum = 'unduhan tidak cocok dengan checksum-nya, jadi tidak ada yang dipasang'
        e_winget = 'Windows ini tidak punya winget, jadi LibreOffice dilewati: unduh dari libreoffice.org'
        put = 'Menaruh printertui di'
        done = 'Halaman tercetak, tinta kering, siap dipakai. Jalankan: printertui'
        rm_cmd = 'Menghapus perintah printertui'
        rm_cfg = 'Menghapus pengaturan dan file sementara'
        bye = 'Kertas sudah kembali ke baki. Sampai jumpa!'
        quit = 'Tenang, tidak ada yang diubah.'
        extras = 'Mau tambahan? Semuanya opsional.'
        check_hint = '↑↓ pindah · spasi tandai · enter lanjut · q keluar'
        lo = 'LibreOffice: cetak Word, Excel, dan file non-PDF lainnya'
        extras_inst = 'Memasang tambahan'
        left_win = 'Printermu tetap seperti semula.'
        start = 'Juga ada di menu Start: PrinterTUI'
        desktop = 'Pintasan PrinterTUI di desktop'
    }
}

# --- pixel art: two pixels per character with half blocks --------------------------------------
# . empty  w paper  k ink  b blue  d dark body  g light body  s slot  G green  r red
$Colors = New-Object Collections.Hashtable ([StringComparer]::Ordinal)
$pairs = 'w 231 k 246 b 69 d 240 g 252 s 234 G 114 r 203' -split ' '
for ($i = 0; $i -lt $pairs.Count; $i += 2) { $Colors[$pairs[$i]] = $pairs[$i + 1] }
$Printer = @(
    '......wwwwwwww......'
    '......wwwwwwww......'
    '..bbbbbbbbbbbbbbbb..'
    '.bbbbbbbbbbbbbbbbbb.'
    '.dggggggggggggggggd.'
    '.dggggggggggggLgMgd.'
    '.dggggggggggggggggd.'
    '.dddddddddddddddddd.'
    '..dssssssssssssssd..'
    '...dddddddddddddd...'
)
$Page = @(
    '...wwwwwwwwwwwwww...'
    '...wbbbbbbbbwwwww...'
    '...wwwwwwwwwwwwww...'
    '...wkkkkkkkkkkkkw...'
    '...wkkkkkkkkwwwww...'
    '...wwwwwwwwwwwwww...'
    '...wkkkkkkkkkkkww...'
    '...wkkkkkkwwwwwww...'
    '...wwwwwwGGwwwwww...'
    '...wwwwwwwwwwwwww...'
)
# crumpled page stuck in the slot, with a red smudge
$Jam = @(
    '...wwkwwwwwkwwww....'
    '..w.wwwrrwwww.ww....'
    '...ww.wwwkww.w.w....'
    '....w.wwkw..w.......'
    '......w..w..........'
)

function Render([string[]]$rows, [int]$pad) {
    $sb = New-Object Text.StringBuilder
    for ($r = 0; $r -lt $rows.Count; $r += 2) {
        [void]$sb.Append(' ' * $pad)
        $top = $rows[$r]
        $bottom = $rows[$r + 1]
        for ($i = 0; $i -lt $top.Length; $i++) {
            $t = [string]$top[$i]
            $u = [string]$bottom[$i]
            if ($t -ceq '.' -and $u -ceq '.') { [void]$sb.Append("$E[0m ") }
            elseif ($t -ceq '.') { [void]$sb.Append("$E[0;38;5;$($Colors[$u])m▄") }
            elseif ($u -ceq '.') { [void]$sb.Append("$E[0;38;5;$($Colors[$t])m▀") }
            else { [void]$sb.Append("$E[38;5;$($Colors[$t]);48;5;$($Colors[$u])m▀") }
        }
        [void]$sb.Append("$E[0m$E[K`n")
    }
    W $sb.ToString()
}

# $n rows of paper out of the slot, two lights, and the indent (2, the printer shakes with 1 and 3);
# drawn over the previous frame, so the art grows and shrinks with the paper
$S.paper = $Page
function Show([int]$n, [string]$l1, [string]$l2, [int]$pad = 2) {
    if ($S.drawn) { W "$E[$($S.drawn)A" }
    $rows = @($Printer | ForEach-Object { $_.Replace('L', $l1).Replace('M', $l2) })
    if ($n -gt 0) { $rows += $S.paper[($S.paper.Count - $n)..($S.paper.Count - 1)] }
    if ($n % 2) { $rows += '....................' }
    Render $rows $pad
    W "$E[J"
    $S.drawn = [int][math]::Floor((11 + $n) / 2)
    Start-Sleep -Milliseconds 90
}

function Blink([int]$n) {
    if ($n % 2) { Show $n 'g' 'G' } else { Show $n 'G' 'g' }
}

function Animate([string]$what, $task) {
    if (-not $tty) { return }
    [Console]::CursorVisible = $false
    # finish and jam carry on from where print stopped
    if ($what -ne 'finish' -and $what -ne 'jam') { $S.drawn = 0 }
    switch ($what) {
        'boot' { Play 'boot'; foreach ($l in 'gg', 'Gg', 'gg', 'Gg', 'GG') { Show 0 $l[0] $l[1] } }
        'print' {
            $S.n = 0
            while (-not $task.IsCompleted) {
                if ($S.n -eq 0) { Play 'print' }
                Blink $S.n
                $S.n = ($S.n + 1) % 11
            }
        }
        'finish' {
            for (; $S.n -le 10; $S.n += 1) { Blink $S.n }
            Play 'done'
            Show 10 'G' 'G'
        }
        'jam' {
            Play 'jam'
            $S.paper = $Jam
            $i = 0
            foreach ($x in 1, 3, 1, 3, 1, 3, 2, 2, 2, 2) {
                if ($i % 2) { Show 5 'g' 'g' $x } else { Show 5 'r' 'r' $x }
                $i++
            }
            Show 5 'r' 'r'
            $S.paper = $Page
        }
        'unprint' {
            Play 'bye'
            for ($n = 10; $n -ge 0; $n--) { if ($n % 2) { Show $n 'g' 'r' } else { Show $n 'r' 'g' } }
            Show 0 'r' 'r'
        }
    }
    [Console]::CursorVisible = $true
}

# --- menu: arrows or j/k, enter, q; with $marks a checklist where space marks --------------------
# Returns the picked index, or -1 for quit, and leaves only the pick on screen.
function Menu([string[]]$items, [int]$sel = 0, [bool[]]$marks = $null) {
    # ($null -ne $marks, not just $marks: an array holding one $false counts as false)
    $n = $items.Count
    $hint = if ($null -ne $marks) { $T.check_hint } else { $T.hint }
    $done = $false
    while (-not $done) {
        for ($i = 0; $i -lt $n; $i++) {
            $box = ''
            if ($null -ne $marks) { $box = if ($marks[$i]) { '[x] ' } else { '[ ] ' } }
            if ($i -eq $sel) { W "  $E[1;38;5;69m▶ $box$($items[$i])$E[0m$E[K`n" }
            else { W "    $E[2m$box$($items[$i])$E[0m$E[K`n" }
        }
        W "`n  $E[2m$hint$E[0m$E[K`n"
        $key = [Console]::ReadKey($true)
        switch ($key.Key) {
            { $_ -eq 'UpArrow' -or $_ -eq 'K' } { if ($sel -gt 0) { $sel--; Play 'blip' } }
            { $_ -eq 'DownArrow' -or $_ -eq 'J' } { if ($sel -lt $n - 1) { $sel++; Play 'blip' } }
            'Spacebar' { if ($null -ne $marks) { $marks[$sel] = -not $marks[$sel] } }
            'Enter' { $done = $true }
            { $_ -eq 'Q' -or $_ -eq 'Escape' } { $sel = -1; $done = $true }
        }
        W "$E[$($n + 2)A"
    }
    W "$E[J"
    if ($sel -ge 0) {
        if ($null -ne $marks) {
            for ($i = 0; $i -lt $n; $i++) { if ($marks[$i]) { W "  $E[1;38;5;114m✓ $($items[$i])$E[0m`n" } }
        } else {
            W "  $E[1;38;5;69m▶ $($items[$sel])$E[0m`n"
        }
    }
    return $sel
}

# --- the actual work ------------------------------------------------------------------------
function Get-Soffice {
    foreach ($p in $env:ProgramFiles, ${env:ProgramFiles(x86)}) {
        if ($p -and (Test-Path (Join-Path $p 'LibreOffice\program\soffice.exe'))) { return $true }
    }
    return $false
}

function New-Shortcut([string]$path) {
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($path)
    $link.TargetPath = $exe
    $link.IconLocation = "$exe,0"
    $link.WorkingDirectory = [Environment]::GetFolderPath('MyDocuments')
    $link.Save()
}

# Downloads the exe and its checksum while the printer prints: $true when installed
function Install-PrinterTUI([bool]$libreoffice, [bool]$onDesktop) {
    if ($libreoffice) {
        Say $T.extras_inst
        # older Windows 10 and LTSC have no winget: say so and install PrinterTUI anyway
        if (-not $preview -and -not (Get-Command winget -ErrorAction SilentlyContinue)) { Fail $T.e_winget }
        else {
            Step 'winget install TheDocumentFoundation.LibreOffice' {
                winget install -e --id TheDocumentFoundation.LibreOffice --accept-package-agreements --accept-source-agreements
            }
        }
    }
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
    # $env:PRINTERTUI_DOWNLOAD points at another folder of release files, for testing a build
    $from = if ($env:PRINTERTUI_DOWNLOAD) { $env:PRINTERTUI_DOWNLOAD } else { "https://github.com/$repo/releases/latest/download" }
    $url = "$from/printertui-windows-$arch.exe"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "printertui-$PID.exe"
    Say "$($T.download) (windows $arch)"
    if ($preview) {
        W "    $E[2m$($T.skip) $url$E[0m`n"
        $task = [Threading.Tasks.Task]::Delay(3000)
    } else {
        $sum = (New-Object Net.WebClient).DownloadStringTaskAsync("$url.sha256")
        $file = (New-Object Net.WebClient).DownloadFileTaskAsync($url, $tmp)
        $task = [Threading.Tasks.Task]::WhenAll([Threading.Tasks.Task[]]@($sum, $file))
    }
    W "`n"
    Animate 'print' $task
    while (-not $task.IsCompleted) { Start-Sleep -Milliseconds 100 }
    # 0 ok, 1 download failed, 2 checksum mismatch
    $status = 0
    if ($preview -eq 'jam') { $status = 1 }
    elseif ($preview -eq 'smudge') { $status = 2 }
    elseif (-not $preview) {
        if ($task.IsFaulted) { $status = 1 }
        elseif ((Get-FileHash $tmp -Algorithm SHA256).Hash -ne ($sum.Result -split '\s+')[0].ToUpper()) { $status = 2 }
    }
    if ($status -eq 0) { Animate 'finish' } else { Animate 'jam' }
    if ($status -ne 0) {
        Remove-Item $tmp -ErrorAction SilentlyContinue
        if ($status -eq 1) { Talk $T.jam -Pet -Color 203; Fail $T.e_net } else { Talk $T.smudge -Pet -Color 203; Fail $T.e_sum }
        return $false
    }
    W "`n"
    Say "$($T.put) $dir"
    Step "move $tmp $exe" {
        New-Item -ItemType Directory -Force $dir | Out-Null
        Move-Item -Force $tmp $exe
    }
    # on the PATH for new terminals, and for this one right away
    Step "PATH += $dir" {
        $path = @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ })
        if ($path -notcontains $dir) { [Environment]::SetEnvironmentVariable('Path', (($path + $dir) -join ';'), 'User') }
        if (($env:Path -split ';') -notcontains $dir) { $env:Path += ";$dir" }
    }
    Step $shortcut { New-Shortcut $shortcut }
    if ($onDesktop) { Step $desktopLink { New-Shortcut $desktopLink } }
    return $true
}

function Uninstall-PrinterTUI {
    Say $T.rm_cmd
    Step "remove $dir, $shortcut, $desktopLink, PATH" {
        Get-Process printertui -ErrorAction SilentlyContinue | Stop-Process -Force
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
        Remove-Item $shortcut, $desktopLink -ErrorAction SilentlyContinue
        $path = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ -and $_ -ne $dir }
        [Environment]::SetEnvironmentVariable('Path', ($path -join ';'), 'User')
    }
    Say $T.rm_cfg
    # settings, and the copy of pdfium the app unpacks
    $data = @((Join-Path $roaming 'printertui'), (Join-Path $base 'printertui'))
    Step "remove $($data -join ', ')" { Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue }
    W "`n"
    Animate 'unprint'
}

# the app speaks the language picked in the menu (without a menu it follows the system's)
function Save-Lang([string]$code) {
    $conf = $configFile
    Step "lang=$code > $conf" {
        New-Item -ItemType Directory -Force (Split-Path $conf) | Out-Null
        $lines = @(if (Test-Path $conf) { [IO.File]::ReadAllLines($conf) | Where-Object { $_ -notmatch '^lang=' } })
        # UTF-8 without the BOM Windows PowerShell adds, which the app would read as part of a key
        [IO.File]::WriteAllLines($conf, [string[]]($lines + "lang=$code"), [Text.UTF8Encoding]::new($false))
    }
}

# --- main -----------------------------------------------------------------------------------
$installed = if ($preview) { $preview -eq 'installed' } else { Test-Path $exe }
$codes = @($Langs.Keys)
# the system's language comes first in the menu
$system = [Globalization.CultureInfo]::CurrentUICulture.TwoLetterISOLanguageName
if ($codes -contains $system) { $codes = @($system) + @($codes | Where-Object { $_ -ne $system }) }
$lang = 0
$T = $Langs[$codes[$lang]]

$picked = $false
try {
    $action = if ($installed) { 'update' } else { 'install' }
    $libreoffice = $false
    $onDesktop = $true
    if ($tty) {
        W "`n  $E[1mPrinterTUI$E[0m`n`n"
        $lang = Menu @($codes | ForEach-Object { $Langs[$_].name }) $lang
        if ($lang -lt 0) { Talk $Langs[$codes[0]].quit; return }
        $T = $Langs[$codes[$lang]]
        $picked = $true
        if ($preview) { W "`n  $E[33m$($T.prev)$E[0m`n" }
        W "`n"
        Animate 'boot'
        if ($installed) {
            Talk $T.hi_old -Pet
            $action = @('quit', 'update', 'uninstall')[(Menu @($T.update, $T.uninstall)) + 1]
        } else {
            Talk $T.hi_new -Pet
            $action = @('quit', 'install')[(Menu @($T.install)) + 1]
        }
        if ($action -eq 'install' -or $action -eq 'update') {
            Talk $T.extras
            # LibreOffice starts marked when it is already there, the desktop shortcut always
            $marks = [bool[]]@((Get-Soffice), $true)
            if ((Menu @($T.lo, $T.desktop) 0 $marks) -lt 0) { $action = 'quit' }
            else { $libreoffice = $marks[0] -and -not (Get-Soffice); $onDesktop = $marks[1] }
        }
        W "`n"
    }
    switch ($action) {
        'quit' { Talk $T.quit }
        'uninstall' {
            Uninstall-PrinterTUI
            Talk $T.bye -Pet
            Say $T.left_win
        }
        default {
            if (Install-PrinterTUI $libreoffice $onDesktop) {
                if ($picked) { Save-Lang $codes[$lang] }
                Talk $T.done
                Say $T.start
            }
        }
    }
} finally {
    if ($tty) { [Console]::CursorVisible = $true }
    [Net.ServicePointManager]::SecurityProtocol = $oldProtocol
    if ($oldEncoding) { try { [Console]::OutputEncoding = $oldEncoding } catch { } }
}
}
