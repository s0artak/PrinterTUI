#!/bin/sh
# PrinterTUI installer for Linux (Arch, Debian, Ubuntu, Fedora, openSUSE and their relatives) and macOS.
# Other Linux systems (image-based ones, other package managers) get PrinterTUI and a hint for the packages.
#   curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh
# Asks for a language, then opens a menu: Install the first time, Update or Uninstall when
# PrinterTUI is already there, then a checklist of optional extras.
# Skip all of it with an argument: ... | sh -s install | update | uninstall
# PRINTERTUI_BIN_DIR=dir puts printertui there instead of /usr/local/bin (or ~/.local/bin without root).
# PRINTERTUI_PREVIEW=fresh|installed|jam|smudge plays the menus and animations without changing anything
# (see installer-preview.sh).
set -eu

REPO=s0artak/PrinterTUI
preview=${PRINTERTUI_PREVIEW:-}
esc=$(printf '\033')
cr=$(printf '\r')

# menus and animations only when a person is watching
tty= saved= tmp= drawn=
if [ -t 1 ] && { : </dev/tty; } 2>/dev/null; then
    tty=1
    saved=$(stty -g </dev/tty)
fi
cleanup() {
    [ -z "$tmp" ] || rm -f "$tmp" "$tmp.sha256"
    [ -z "$saved" ] || stty "$saved" </dev/tty
    [ -z "$tty" ] || printf '\033[?25h'
}
trap cleanup EXIT
trap 'exit 130' INT TERM

say() { printf '  \033[38;5;69m▸\033[0m %s\n' "$*"; }
# The printer's speech bubble, as in the app. "talk TEXT pet" puts it beside the printer just
# drawn, its tail pointing at it; a third argument is the border color (203: red, for errors).
talk() {
    if [ -z "$tty" ]; then
        printf '\n  %s\n\n' "$1"
        return
    fi
    cols=$(stty size </dev/tty 2>/dev/null | cut -d' ' -f2)
    left=2
    if [ "${2:-}" = pet ] && [ -n "$drawn" ] && [ "${cols:-80}" -ge 64 ]; then left=24; fi
    max=$((${cols:-80} - left - 5))
    [ $max -le 60 ] || max=60
    lines=$(wrap "$1" $max)
    n=$(printf '%s\n' "$lines" | wc -l)
    w=$(printf '%s\n' "$lines" | cut -f1 | sort -n | tail -n 1)
    bar=$(printf '%*s' $((w + 2)) '' | sed 's/ /─/g')
    c="\033[38;5;${3:-69}m" go="\033[$((left + 1))G"
    # beside the printer: back up to its second row, so the tail meets its lights
    if [ $left -gt 2 ]; then printf '\033[%dA' $((drawn - 1)); else echo; fi
    printf "$go$c╭%s╮\033[0m\n" "$bar"
    first=1
    printf '%s\n' "$lines" | while IFS='	' read -r lw line; do
        edge=│
        if [ $left -gt 2 ] && [ -n "$first" ]; then edge=◀; fi
        printf "$go$c%s\033[0m %s%*s $c│\033[0m\n" "$edge" "$line" $((w - lw)) ''
        first=
    done
    printf "$go$c╰%s╯\033[0m\n" "$bar"
    if [ $left -gt 2 ] && [ $((n + 3)) -lt "$drawn" ]; then printf '\033[%dB' $((drawn - n - 3)); fi
    echo
    drawn=
}

# --- sounds: the app's (see src/sound.rs), synthesized here, as there are no files to ship ---
# The app's volume when it has one (an update keeps it, muted too), else its gentle default.
vol=$(sed -n 's/^volume=\([0-9][0-9]*\)$/\1/p' "${XDG_CONFIG_HOME:-$HOME/.config}/printertui/config" 2>/dev/null | tail -n 1)
vol=${vol:-50}

# Plays sound $1 (boot blip print done jam bye) in the background, when a person is listening.
play() {
    [ -n "$tty" ] && [ "$vol" -gt 0 ] || return 0
    # in the user's own temp folder, as the app's: one someone else made first (a link in it could
    # send the sound over another file) means no sounds
    d="${TMPDIR:-/tmp}/printertui-$(id -u)"
    mkdir -m 700 "$d" 2>/dev/null || :
    [ -d "$d" ] && [ ! -L "$d" ] && [ -O "$d" ] && chmod 700 "$d" 2>/dev/null || return 0
    # v2: the sounds' version, as in src/sound.rs
    f="$d/install-sounds/$1-$vol-v2.wav"
    if [ ! -s "$f" ]; then
        mkdir -p "${f%/*}" 2>/dev/null || return 0
        # shellcheck disable=SC2059 # the format is the sound's bytes, as octal escapes
        printf "$(synth "$1" "$vol")" >"$f" 2>/dev/null || return 0
    fi
    for p in afplay pw-play paplay 'aplay -q'; do
        if command -v "${p%% *}" >/dev/null 2>&1; then
            $p "$f" >/dev/null 2>&1 &
            return 0
        fi
    done
}

# Sound $1 at volume $2 as an 8-bit WAV file, written as printf octal escapes (POSIX, so the
# same in every sh and awk).
synth() {
    LC_ALL=C awk -v name="$1" -v vol="$2" '
        function note(f, secs, amp,   i, n, t, env) {
            n = int(secs * R)
            for (i = 0; i < n; i++) {
                t = i / R; env = (t * 400 < 1 ? t * 400 : 1) * exp(-t * 9)
                s[N++] = amp * env * (sin(TAU * f * t) + 0.3 * sin(3 * TAU * f * t)) / 1.3
            }
        }
        function silence(secs,   i) { for (i = 0; i < int(secs * R); i++) s[N++] = 0 }
        function fract(x) { return x - int(x) }
        function le(n, k,   i) { for (i = 0; i < k; i++) { printf "\\%03o", n % 256; n = int(n / 256) } }
        function text(t,   i) { for (i = 1; i <= length(t); i++) printf "%s", substr(t, i, 1) }
        BEGIN {
            R = 11025; TAU = 6.283185307; N = 0; srand(7)
            if (name == "boot") { note(523.25, 0.12, 0.56); note(659.25, 0.12, 0.56); note(783.99, 0.12, 0.56) }
            else if (name == "bye") { note(783.99, 0.14, 0.56); note(659.25, 0.14, 0.56); note(523.25, 0.2, 0.56) }
            else if (name == "blip") note(1318.5, 0.05, 0.35)
            else if (name == "done") { note(880, 0.12, 0.7); note(1318.5, 0.3, 0.7) }
            else if (name == "print") {
                for (i = 0; i < int(1.1 * R); i++) {
                    t = i / R; fade = (t * 20 < 1 ? t * 20 : 1) * ((1.1 - t) * 20 < 1 ? (1.1 - t) * 20 : 1)
                    saw = 2 * fract(t * 70 + 0.5 * sin(t * 3)) - 1
                    s[N++] = 0.55 * fade * (0.55 + 0.45 * sin(TAU * 7 * t)) * (0.7 * saw + 0.3 * (2 * rand() - 1))
                }
                last = 0; n = int(0.18 * R)
                for (i = 0; i < n; i++) { x = 2 * rand() - 1; s[N++] = 0.12 * (1 - i / n) ^ 2 * (x - last); last = x }
            } else if (name == "jam") {
                for (k = 0; k < 3; k++) {
                    for (i = 0; i < int(0.16 * R); i++) {
                        t = i / R; env = (t * 200 < 1 ? t * 200 : 1) * (1 - t / 0.16)
                        s[N++] = 0.42 * env * (0.8 * (fract(t * 95) < 0.5 ? 1 : -1) + 0.2 * (2 * rand() - 1))
                    }
                    silence(0.05)
                }
                ph = 0
                for (i = 0; i < int(0.35 * R); i++) { t = i / R; ph += TAU * (300 - 500 * t) / R; s[N++] = 0.5 * (1 - t / 0.35) * sin(ph) }
            }
            gain = (vol / 100) ^ 2
            text("RIFF"); le(36 + N, 4); text("WAVEfmt "); le(16, 4); le(1, 2); le(1, 2); le(R, 4); le(R, 4); le(1, 2); le(8, 2)
            text("data"); le(N, 4)
            for (i = 0; i < N; i++) {
                v = int(128 + 127 * s[i] * gain + 0.5)
                printf "\\%03o", (v < 0 ? 0 : v > 255 ? 255 : v)
            }
        }'
}

# Word-wraps $1 into lines of at most $2 screen columns, each as "columns<TAB>text". Counted by
# bytes (the same in every awk): Chinese takes two columns, the marks that Hindi, Bengali and
# Arabic draw over the letter before take none.
wrap() {
    printf '%s\n' "$1" | LC_ALL=C awk -v max="$2" '
        BEGIN { for (i = 0; i < 256; i++) ord[sprintf("%c", i)] = i }
        function mark(u) {
            return (u >= 768 && u <= 879) || (u >= 1611 && u <= 1631) || u == 1648 ||
                (u >= 2304 && u <= 2306) || u == 2362 || u == 2364 || (u >= 2369 && u <= 2376) || u == 2381 ||
                (u >= 2385 && u <= 2391) || (u >= 2402 && u <= 2403) || u == 2433 || u == 2492 ||
                (u >= 2497 && u <= 2500) || u == 2509 || (u >= 2530 && u <= 2531) || u == 8204 || u == 8205
        }
        function wide(u) {
            return (u >= 4352 && u <= 4447) || (u >= 11904 && u <= 42191) || (u >= 44032 && u <= 55203) ||
                (u >= 63744 && u <= 64255) || (u >= 65040 && u <= 65135) || (u >= 65280 && u <= 65376) || (u >= 65504 && u <= 65510)
        }
        function cols(s,   i, n, b, u, w) {
            n = length(s); w = 0
            for (i = 1; i <= n;) {
                b = ord[substr(s, i, 1)]
                if (b < 128) { w++; i++; continue }
                if (b < 224) { u = (b % 32) * 64 + ord[substr(s, i + 1, 1)] % 64; i += 2 }
                else if (b < 240) { u = ((b % 16) * 64 + ord[substr(s, i + 1, 1)] % 64) * 64 + ord[substr(s, i + 2, 1)] % 64; i += 3 }
                else { w += 2; i += 4; continue }
                if (wide(u)) w += 2; else if (!mark(u)) w++
            }
            return w
        }
        {
            n = split($0, word, " "); line = ""; lw = 0
            for (j = 1; j <= n; j++) {
                ww = cols(word[j])
                if (lw > 0 && lw + 1 + ww > max) { print lw "\t" line; line = word[j]; lw = ww }
                else if (lw > 0) { line = line " " word[j]; lw += 1 + ww }
                else { line = word[j]; lw = ww }
            }
            print lw "\t" line
        }'
}
die() { printf '  \033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
# a problem that does not stop the install; $2, when given, is a line to copy (a command, a setting)
warn() {
    printf '  \033[33m! %s\033[0m\n' "$1"
    [ -z "${2:-}" ] || printf '      \033[1m%s\033[0m\n' "$2"
}
# every command that changes the system goes through run, so the preview can skip it
run() {
    if [ -n "$preview" ]; then
        printf '    \033[2m%s %s\033[0m\n' "$T_skip" "$*"
        sleep 0.3
    else
        "$@"
    fi
}

# Runs a command as root: as it is when we are root, else with sudo or doas. Without them (or when
# sudo refuses the password) only this step fails, with a message (none with $quiet set, when the
# caller has another way).
asked= noroot= quiet=
as_root() {
    if [ "$(id -u)" = 0 ]; then
        run "$@"
    elif [ -z "$noroot" ] && command -v sudo >/dev/null; then
        # the password once, up front: refused, the other steps that need root are skipped too
        if [ -z "$preview$asked" ]; then
            asked=1
            if ! sudo -v; then
                noroot=1
                as_root "$@"
                return
            fi
        fi
        run sudo "$@"
    elif command -v doas >/dev/null; then
        run doas "$@"
    else
        [ -n "$quiet" ] || warn "$T_no_root" "$*"
        return 1
    fi
}
# as_root can work: we are root, or sudo (not refused yet) or doas is there
rootable() { [ "$(id -u)" = 0 ] || { [ -z "$noroot" ] && command -v sudo >/dev/null; } || command -v doas >/dev/null; }

# --- languages: the 10 most spoken (Ethnologue), each sets every T_ text -----
LANGS="en zh hi es ar fr bn pt ru id"

lang_en() {
    T_name="🇬🇧 English"
    T_hint="↑↓ move · enter pick · q quit"
    T_prev="(preview: nothing is installed or removed)"
    T_skip="preview, skipped:"
    T_hi_new="Hi! Looks like we haven't met. Warm up the printer?"
    T_hi_old="Oh, you already have me. Fresh copy, or shall I pack up?"
    T_install="Install" T_update="Update" T_uninstall="Uninstall"
    T_deps_linux="Installing CUPS (sudo may ask for your password)"
    T_cups="Starting CUPS"
    T_download="Downloading PrinterTUI"
    T_jam="Paper jam! The download didn't come through."
    T_put="Putting printertui in"
    T_done="Page printed, ink dry, ready to go. Run: printertui"
    T_rm_cmd="Removing the printertui command"
    T_rm_cfg="Removing settings and temporary files"
    T_bye="Paper's back in the tray. Bye for now!"
    T_left="CUPS and your printers were left as they are."
    T_quit="No worries, nothing was touched."
    T_e_os="needs Linux or macOS"
    T_e_brew="Homebrew is needed first: https://brew.sh"
    T_e_net="check your internet connection and run the installer again"
    T_extras="Want some extras? All optional."
    T_check_hint="↑↓ move · space mark · enter continue · q quit"
    T_lo="LibreOffice: print Word, Excel and other non-PDF files"
    T_sane="SANE: USB scanners that aren't part of a printer"
    T_ocr="Tesseract: searchable text in scanned PDFs (OCR)"
    T_extras_inst="Installing the extras"
    T_smudge="Smudged page! Something's off with that download."
    T_e_sum="the download doesn't match its checksum, so nothing was installed"
    T_e_arch="there's no ready-made PrinterTUI for this processor; build it with: cargo install --git https://github.com/$REPO"
    T_e_404="there's no ready-made PrinterTUI for this system yet (404); build it with: cargo install --git https://github.com/$REPO"
    T_no_root="This needs root, and neither sudo nor doas let me in, so I skipped it:"
    T_deps_fail="CUPS couldn't be installed (see above), so PrinterTUI goes in without it; printing works once CUPS is there"
    T_own_pkgs="I don't install packages on this system: add CUPS's client tools (lp, lpstat, lpq), poppler's pdftoppm and any extras you picked with its own tools"
    T_path="printertui's folder isn't on your PATH yet: add this line to ~/.profile (or your shell's startup file) and open a new terminal"
    T_mdns="To find network printers by name, put mdns_minimal [NOTFOUND=return] before resolve and dns on the hosts line of /etc/nsswitch.conf:"
}

lang_zh() {
    T_name="🇨🇳 中文"
    T_hint="↑↓ 移动 · 回车 选择 · q 退出"
    T_prev="（预览：不会安装或删除任何东西）"
    T_skip="预览，已跳过："
    T_hi_new="你好！我们好像还没见过面。要预热打印机吗？"
    T_hi_old="哦，我已经在这儿了。重新装一份，还是我收拾东西走人？"
    T_install="安装" T_update="更新" T_uninstall="卸载"
    T_deps_linux="正在安装 CUPS（sudo 可能会要求输入密码）"
    T_cups="正在启动 CUPS"
    T_download="正在下载 PrinterTUI"
    T_jam="卡纸了！下载没有成功。"
    T_put="正在把 printertui 放到"
    T_done="页面已打印，墨迹已干，一切就绪。运行：printertui"
    T_rm_cmd="正在删除 printertui 命令"
    T_rm_cfg="正在删除设置和临时文件"
    T_bye="纸已放回纸盒。再见！"
    T_left="CUPS 和你的打印机都保持原样。"
    T_quit="没关系，什么都没改动。"
    T_e_os="需要 Linux 或 macOS"
    T_e_brew="需要先安装 Homebrew：https://brew.sh"
    T_e_net="请检查网络连接后重新运行安装程序"
    T_extras="要来点附加组件吗？全部可选。"
    T_check_hint="↑↓ 移动 · 空格 勾选 · 回车 继续 · q 退出"
    T_lo="LibreOffice：打印 Word、Excel 等非 PDF 文件"
    T_sane="SANE：不属于打印机的 USB 扫描仪"
    T_ocr="Tesseract：让扫描的 PDF 可搜索文字（OCR）"
    T_extras_inst="正在安装附加组件"
    T_smudge="页面印花了！这次下载有问题。"
    T_e_sum="下载的文件与校验和不符，什么都没有安装"
    T_e_arch="这种处理器没有现成的 PrinterTUI，可以自己编译：cargo install --git https://github.com/$REPO"
    T_e_404="这个系统还没有现成的 PrinterTUI（404），可以自己编译：cargo install --git https://github.com/$REPO"
    T_no_root="这一步需要 root 权限，但 sudo 和 doas 都没让我进去，所以跳过了："
    T_deps_fail="CUPS 没能安装（见上方），PrinterTUI 照样安装；装好 CUPS 之后就能打印"
    T_own_pkgs="我不在这个系统上安装软件包：请用系统自己的工具添加 CUPS 客户端工具（lp、lpstat、lpq）、poppler 的 pdftoppm 以及你选的附加组件"
    T_path="printertui 所在的文件夹还不在你的 PATH 里：把这一行加到 ~/.profile（或你的 shell 启动文件）里，然后打开一个新终端"
    T_mdns="要按名称找到网络打印机，请在 /etc/nsswitch.conf 的 hosts 行里，把 mdns_minimal [NOTFOUND=return] 放在 resolve 和 dns 前面："
}

lang_hi() {
    T_name="🇮🇳 हिन्दी"
    T_hint="↑↓ चुनें · enter पक्का करें · q बाहर"
    T_prev="(पूर्वावलोकन: कुछ भी इंस्टॉल या हटाया नहीं जाता)"
    T_skip="पूर्वावलोकन, छोड़ा गया:"
    T_hi_new="नमस्ते! लगता है हम पहली बार मिल रहे हैं। प्रिंटर गरम करें?"
    T_hi_old="अरे, मैं तो पहले से यहाँ हूँ। नई कॉपी, या मैं अपना सामान बाँधूँ?"
    T_install="इंस्टॉल करें" T_update="अपडेट करें" T_uninstall="अनइंस्टॉल करें"
    T_deps_linux="CUPS इंस्टॉल हो रहा है (sudo पासवर्ड माँग सकता है)"
    T_cups="CUPS शुरू हो रहा है"
    T_download="PrinterTUI डाउनलोड हो रहा है"
    T_jam="पेपर जाम! डाउनलोड नहीं हो पाया।"
    T_put="printertui को यहाँ रखा जा रहा है:"
    T_done="पेज छप गया, स्याही सूख गई, सब तैयार। चलाएँ: printertui"
    T_rm_cmd="printertui कमांड हटाई जा रही है"
    T_rm_cfg="सेटिंग्स और अस्थायी फ़ाइलें हटाई जा रही हैं"
    T_bye="कागज़ वापस ट्रे में। फिर मिलेंगे!"
    T_left="CUPS और आपके प्रिंटर जैसे थे वैसे ही हैं।"
    T_quit="कोई बात नहीं, कुछ भी नहीं बदला।"
    T_e_os="Linux या macOS चाहिए"
    T_e_brew="पहले Homebrew चाहिए: https://brew.sh"
    T_e_net="इंटरनेट कनेक्शन जाँचें और इंस्टॉलर फिर से चलाएँ"
    T_extras="कुछ अतिरिक्त चाहिए? सब वैकल्पिक हैं।"
    T_check_hint="↑↓ चुनें · space निशान · enter आगे · q बाहर"
    T_lo="LibreOffice: Word, Excel और दूसरी गैर-PDF फ़ाइलें छापें"
    T_sane="SANE: USB स्कैनर जो प्रिंटर का हिस्सा नहीं हैं"
    T_ocr="Tesseract: स्कैन की गई PDF में खोजने लायक टेक्स्ट (OCR)"
    T_extras_inst="अतिरिक्त चीज़ें इंस्टॉल हो रही हैं"
    T_smudge="पेज पर धब्बा! इस डाउनलोड में कुछ गड़बड़ है।"
    T_e_sum="डाउनलोड चेकसम से मेल नहीं खाता, इसलिए कुछ भी इंस्टॉल नहीं हुआ"
    T_e_arch="इस प्रोसेसर के लिए तैयार PrinterTUI नहीं है; इसे खुद बनाएँ: cargo install --git https://github.com/$REPO"
    T_e_404="इस सिस्टम के लिए अभी तैयार PrinterTUI नहीं है (404); इसे खुद बनाएँ: cargo install --git https://github.com/$REPO"
    T_no_root="इसके लिए root चाहिए, पर sudo या doas से अनुमति नहीं मिली, इसलिए इसे छोड़ दिया:"
    T_deps_fail="CUPS इंस्टॉल नहीं हो पाया (ऊपर देखें), फिर भी PrinterTUI इंस्टॉल हो रहा है; CUPS आने पर छपाई चलेगी"
    T_own_pkgs="इस सिस्टम पर मैं पैकेज इंस्टॉल नहीं करता: CUPS के क्लाइंट टूल (lp, lpstat, lpq), poppler का pdftoppm और चुनी हुई अतिरिक्त चीज़ें सिस्टम के अपने टूल से जोड़ें"
    T_path="printertui का फ़ोल्डर अभी आपके PATH में नहीं है: यह पंक्ति ~/.profile (या अपने shell की स्टार्टअप फ़ाइल) में जोड़ें और नया टर्मिनल खोलें"
    T_mdns="नेटवर्क प्रिंटर नाम से ढूँढने के लिए /etc/nsswitch.conf की hosts पंक्ति में resolve और dns से पहले mdns_minimal [NOTFOUND=return] लगाएँ:"
}

lang_es() {
    T_name="🇪🇸 Español"
    T_hint="↑↓ mover · enter elegir · q salir"
    T_prev="(vista previa: no se instala ni se borra nada)"
    T_skip="vista previa, omitido:"
    T_hi_new="¡Hola! Parece que no nos conocemos. ¿Calentamos la impresora?"
    T_hi_old="Anda, ya me tienes instalada. ¿Copia nueva o recojo mis cosas?"
    T_install="Instalar" T_update="Actualizar" T_uninstall="Desinstalar"
    T_deps_linux="Instalando CUPS (sudo puede pedirte la contraseña)"
    T_cups="Arrancando CUPS"
    T_download="Descargando PrinterTUI"
    T_jam="¡Atasco de papel! La descarga no ha llegado."
    T_put="Poniendo printertui en"
    T_done="Página impresa, tinta seca, todo listo. Ejecuta: printertui"
    T_rm_cmd="Quitando el comando printertui"
    T_rm_cfg="Borrando ajustes y archivos temporales"
    T_bye="El papel ha vuelto a la bandeja. ¡Hasta pronto!"
    T_left="CUPS y tus impresoras se quedan como estaban."
    T_quit="Tranquilo, no he tocado nada."
    T_e_os="hace falta Linux o macOS"
    T_e_brew="primero hace falta Homebrew: https://brew.sh"
    T_e_net="revisa tu conexión a internet y vuelve a ejecutar el instalador"
    T_extras="¿Unos extras? Todos son opcionales."
    T_check_hint="↑↓ mover · espacio marcar · enter seguir · q salir"
    T_lo="LibreOffice: imprimir Word, Excel y otros archivos que no son PDF"
    T_sane="SANE: escáneres USB que no forman parte de una impresora"
    T_ocr="Tesseract: texto buscable en los PDF escaneados (OCR)"
    T_extras_inst="Instalando los extras"
    T_smudge="¡Página emborronada! Algo raro pasa con esa descarga."
    T_e_sum="la descarga no coincide con su checksum, así que no he instalado nada"
    T_e_arch="no hay PrinterTUI listo para este procesador; compílalo con: cargo install --git https://github.com/$REPO"
    T_e_404="todavía no hay PrinterTUI listo para este sistema (404); compílalo con: cargo install --git https://github.com/$REPO"
    T_no_root="Esto necesita root y ni sudo ni doas me han dejado, así que me lo he saltado:"
    T_deps_fail="No se ha podido instalar CUPS (mira arriba), así que PrinterTUI se instala sin él; podrás imprimir cuando CUPS esté"
    T_own_pkgs="En este sistema no instalo paquetes: añade con sus propias herramientas los clientes de CUPS (lp, lpstat, lpq), el pdftoppm de poppler y los extras que hayas elegido"
    T_path="La carpeta de printertui aún no está en tu PATH: añade esta línea a ~/.profile (o al archivo de inicio de tu shell) y abre otra terminal"
    T_mdns="Para encontrar impresoras de red por su nombre, pon mdns_minimal [NOTFOUND=return] antes de resolve y dns en la línea hosts de /etc/nsswitch.conf:"
}

lang_ar() {
    T_name="🇸🇦 العربية"
    T_hint="↑↓ تنقّل · enter اختيار · q خروج"
    T_prev="(معاينة: لا يُثبَّت ولا يُحذف شيء)"
    T_skip="معاينة، تم التخطي:"
    T_hi_new="أهلاً! يبدو أننا لم نلتقِ من قبل. نسخّن الطابعة؟"
    T_hi_old="أوه، أنا مثبّت بالفعل. نسخة جديدة، أم أحزم أغراضي؟"
    T_install="تثبيت" T_update="تحديث" T_uninstall="إزالة"
    T_deps_linux="جارٍ تثبيت CUPS (قد يطلب sudo كلمة المرور)"
    T_cups="جارٍ تشغيل CUPS"
    T_download="جارٍ تنزيل PrinterTUI"
    T_jam="انحشار الورق! لم يكتمل التنزيل."
    T_put="جارٍ وضع printertui في"
    T_done="طُبعت الصفحة وجفّ الحبر، كل شيء جاهز. شغّل: printertui"
    T_rm_cmd="جارٍ حذف أمر printertui"
    T_rm_cfg="جارٍ حذف الإعدادات والملفات المؤقتة"
    T_bye="عاد الورق إلى الدرج. إلى اللقاء!"
    T_left="بقيت CUPS وطابعاتك كما هي."
    T_quit="لا بأس، لم يتغير شيء."
    T_e_os="يلزم Linux أو macOS"
    T_e_brew="يلزم Homebrew أولاً: https://brew.sh"
    T_e_net="تحقّق من اتصالك بالإنترنت ثم أعد تشغيل المثبّت"
    T_extras="هل تريد بعض الإضافات؟ كلها اختيارية."
    T_check_hint="↑↓ تنقّل · space تحديد · enter متابعة · q خروج"
    T_lo="LibreOffice: طباعة ملفات Word و Excel وغيرها من غير PDF"
    T_sane="SANE: ماسحات USB ليست جزءاً من طابعة"
    T_ocr="Tesseract: نص قابل للبحث في ملفات PDF الممسوحة (OCR)"
    T_extras_inst="جارٍ تثبيت الإضافات"
    T_smudge="صفحة ملطّخة! هناك خطب ما في هذا التنزيل."
    T_e_sum="الملف المنزّل لا يطابق المجموع الاختباري، لذلك لم يُثبَّت شيء"
    T_e_arch="لا توجد نسخة جاهزة من PrinterTUI لهذا المعالج؛ ابنِها بنفسك: cargo install --git https://github.com/$REPO"
    T_e_404="لا توجد بعد نسخة جاهزة من PrinterTUI لهذا النظام (404)؛ ابنِها بنفسك: cargo install --git https://github.com/$REPO"
    T_no_root="هذه الخطوة تحتاج صلاحيات root، ولم يسمح لي sudo ولا doas، لذلك تخطّيتها:"
    T_deps_fail="تعذّر تثبيت CUPS (انظر أعلاه)، لذا يُثبَّت PrinterTUI بدونه؛ ستعمل الطباعة بعد تثبيت CUPS"
    T_own_pkgs="لا أثبّت حزماً على هذا النظام: أضف بأدواته الخاصة أدوات عميل CUPS (lp و lpstat و lpq) و pdftoppm من poppler وأي إضافات اخترتها"
    T_path="مجلد printertui ليس في PATH بعد: أضف هذا السطر إلى ~/.profile (أو ملف بدء الصدفة لديك) ثم افتح طرفية جديدة"
    T_mdns="للعثور على طابعات الشبكة بأسمائها، ضع mdns_minimal [NOTFOUND=return] قبل resolve و dns في سطر hosts من /etc/nsswitch.conf:"
}

lang_fr() {
    T_name="🇫🇷 Français"
    T_hint="↑↓ bouger · entrée choisir · q quitter"
    T_prev="(aperçu : rien n'est installé ni supprimé)"
    T_skip="aperçu, ignoré :"
    T_hi_new="Salut ! On ne se connaît pas encore. On fait chauffer l'imprimante ?"
    T_hi_old="Oh, je suis déjà là. Une copie toute neuve, ou je fais mes valises ?"
    T_install="Installer" T_update="Mettre à jour" T_uninstall="Désinstaller"
    T_deps_linux="Installation de CUPS (sudo peut demander votre mot de passe)"
    T_cups="Démarrage de CUPS"
    T_download="Téléchargement de PrinterTUI"
    T_jam="Bourrage papier ! Le téléchargement n'a pas abouti."
    T_put="Installation de printertui dans"
    T_done="Page imprimée, encre sèche, tout est prêt. Lancez : printertui"
    T_rm_cmd="Suppression de la commande printertui"
    T_rm_cfg="Suppression des réglages et fichiers temporaires"
    T_bye="Le papier est retourné dans le bac. À bientôt !"
    T_left="CUPS et vos imprimantes restent tels quels."
    T_quit="Pas de souci, rien n'a été modifié."
    T_e_os="il faut Linux ou macOS"
    T_e_brew="Homebrew est nécessaire d'abord : https://brew.sh"
    T_e_net="vérifiez votre connexion internet puis relancez l'installateur"
    T_extras="Quelques extras ? Tous optionnels."
    T_check_hint="↑↓ bouger · espace cocher · entrée continuer · q quitter"
    T_lo="LibreOffice : imprimer Word, Excel et autres fichiers non PDF"
    T_sane="SANE : scanners USB qui ne font pas partie d'une imprimante"
    T_ocr="Tesseract : texte cherchable dans les PDF scannés (OCR)"
    T_extras_inst="Installation des extras"
    T_smudge="Page tachée ! Ce téléchargement a un souci."
    T_e_sum="le téléchargement ne correspond pas à sa somme de contrôle, rien n'a été installé"
    T_e_arch="pas de PrinterTUI tout prêt pour ce processeur ; compilez-le avec : cargo install --git https://github.com/$REPO"
    T_e_404="pas encore de PrinterTUI tout prêt pour ce système (404) ; compilez-le avec : cargo install --git https://github.com/$REPO"
    T_no_root="Il faut être root, et ni sudo ni doas ne m'ont laissé faire, alors j'ai sauté cette étape :"
    T_deps_fail="CUPS n'a pas pu être installé (voir plus haut), PrinterTUI s'installe quand même ; l'impression marchera une fois CUPS en place"
    T_own_pkgs="Je n'installe pas de paquets sur ce système : ajoutez avec ses propres outils les clients CUPS (lp, lpstat, lpq), le pdftoppm de poppler et les extras choisis"
    T_path="Le dossier de printertui n'est pas encore dans votre PATH : ajoutez cette ligne à ~/.profile (ou au fichier de démarrage de votre shell) puis ouvrez un nouveau terminal"
    T_mdns="Pour trouver les imprimantes réseau par leur nom, mettez mdns_minimal [NOTFOUND=return] avant resolve et dns sur la ligne hosts de /etc/nsswitch.conf :"
}

lang_bn() {
    T_name="🇧🇩 বাংলা"
    T_hint="↑↓ সরান · enter বাছুন · q বের হন"
    T_prev="(প্রিভিউ: কিছুই ইনস্টল বা মোছা হয় না)"
    T_skip="প্রিভিউ, বাদ দেওয়া হয়েছে:"
    T_hi_new="হ্যালো! মনে হচ্ছে আমাদের আগে দেখা হয়নি। প্রিন্টারটা গরম করব?"
    T_hi_old="আরে, আমি তো আগেই আছি। নতুন কপি, নাকি আমি গুছিয়ে চলে যাব?"
    T_install="ইনস্টল" T_update="আপডেট" T_uninstall="আনইনস্টল"
    T_deps_linux="CUPS ইনস্টল হচ্ছে (sudo পাসওয়ার্ড চাইতে পারে)"
    T_cups="CUPS চালু হচ্ছে"
    T_download="PrinterTUI ডাউনলোড হচ্ছে"
    T_jam="কাগজ আটকে গেছে! ডাউনলোড হয়নি।"
    T_put="printertui রাখা হচ্ছে:"
    T_done="পাতা ছাপা হয়েছে, কালি শুকিয়েছে, সব তৈরি। চালান: printertui"
    T_rm_cmd="printertui কমান্ড সরানো হচ্ছে"
    T_rm_cfg="সেটিংস আর অস্থায়ী ফাইল মোছা হচ্ছে"
    T_bye="কাগজ আবার ট্রেতে। আবার দেখা হবে!"
    T_left="CUPS আর আপনার প্রিন্টার যেমন ছিল তেমনই আছে।"
    T_quit="চিন্তা নেই, কিছুই বদলানো হয়নি।"
    T_e_os="Linux অথবা macOS লাগবে"
    T_e_brew="আগে Homebrew লাগবে: https://brew.sh"
    T_e_net="ইন্টারনেট সংযোগ দেখে নিন, তারপর ইনস্টলার আবার চালান"
    T_extras="কিছু বাড়তি জিনিস চান? সবই ঐচ্ছিক।"
    T_check_hint="↑↓ সরান · space চিহ্ন · enter এগোন · q বের হন"
    T_lo="LibreOffice: Word, Excel আর অন্য PDF-নয় এমন ফাইল ছাপুন"
    T_sane="SANE: প্রিন্টারের অংশ নয় এমন USB স্ক্যানার"
    T_ocr="Tesseract: স্ক্যান করা PDF-এ খোঁজা যায় এমন লেখা (OCR)"
    T_extras_inst="বাড়তি জিনিস ইনস্টল হচ্ছে"
    T_smudge="পাতায় দাগ! এই ডাউনলোডে কিছু গোলমাল আছে।"
    T_e_sum="ডাউনলোড চেকসামের সাথে মেলেনি, তাই কিছুই ইনস্টল হয়নি"
    T_e_arch="এই প্রসেসরের জন্য তৈরি PrinterTUI নেই; নিজে বানিয়ে নিন: cargo install --git https://github.com/$REPO"
    T_e_404="এই সিস্টেমের জন্য এখনও তৈরি PrinterTUI নেই (404); নিজে বানিয়ে নিন: cargo install --git https://github.com/$REPO"
    T_no_root="এর জন্য root লাগে, কিন্তু sudo বা doas কেউই ঢুকতে দেয়নি, তাই এটা বাদ দিলাম:"
    T_deps_fail="CUPS ইনস্টল করা যায়নি (উপরে দেখুন), তবু PrinterTUI ইনস্টল হচ্ছে; CUPS এলে ছাপা যাবে"
    T_own_pkgs="এই সিস্টেমে আমি প্যাকেজ ইনস্টল করি না: CUPS-এর ক্লায়েন্ট টুল (lp, lpstat, lpq), poppler-এর pdftoppm আর বেছে নেওয়া বাড়তি জিনিস সিস্টেমের নিজের টুল দিয়ে যোগ করুন"
    T_path="printertui-এর ফোল্ডার এখনও আপনার PATH-এ নেই: এই লাইনটি ~/.profile-এ (বা আপনার shell-এর স্টার্টআপ ফাইলে) যোগ করে নতুন টার্মিনাল খুলুন"
    T_mdns="নেটওয়ার্ক প্রিন্টার নাম দিয়ে খুঁজে পেতে /etc/nsswitch.conf-এর hosts লাইনে resolve আর dns-এর আগে mdns_minimal [NOTFOUND=return] বসান:"
}

lang_pt() {
    T_name="🇧🇷 Português"
    T_hint="↑↓ mover · enter escolher · q sair"
    T_prev="(prévia: nada é instalado nem removido)"
    T_skip="prévia, ignorado:"
    T_hi_new="Oi! Parece que ainda não nos conhecemos. Vamos esquentar a impressora?"
    T_hi_old="Ah, eu já estou aqui. Cópia nova, ou arrumo as malas?"
    T_install="Instalar" T_update="Atualizar" T_uninstall="Desinstalar"
    T_deps_linux="Instalando CUPS (o sudo pode pedir sua senha)"
    T_cups="Iniciando o CUPS"
    T_download="Baixando o PrinterTUI"
    T_jam="Papel atolado! O download não chegou."
    T_put="Colocando o printertui em"
    T_done="Página impressa, tinta seca, tudo pronto. Execute: printertui"
    T_rm_cmd="Removendo o comando printertui"
    T_rm_cfg="Removendo configurações e arquivos temporários"
    T_bye="O papel voltou para a bandeja. Até mais!"
    T_left="CUPS e suas impressoras ficaram como estavam."
    T_quit="Tudo bem, nada foi alterado."
    T_e_os="é preciso Linux ou macOS"
    T_e_brew="primeiro é preciso o Homebrew: https://brew.sh"
    T_e_net="verifique sua conexão com a internet e rode o instalador de novo"
    T_extras="Quer uns extras? Todos opcionais."
    T_check_hint="↑↓ mover · espaço marcar · enter continuar · q sair"
    T_lo="LibreOffice: imprimir Word, Excel e outros arquivos que não são PDF"
    T_sane="SANE: scanners USB que não fazem parte de uma impressora"
    T_ocr="Tesseract: texto pesquisável nos PDFs escaneados (OCR)"
    T_extras_inst="Instalando os extras"
    T_smudge="Página borrada! Tem algo errado com esse download."
    T_e_sum="o download não bate com o checksum, então nada foi instalado"
    T_e_arch="não há PrinterTUI pronto para este processador; compile com: cargo install --git https://github.com/$REPO"
    T_e_404="ainda não há PrinterTUI pronto para este sistema (404); compile com: cargo install --git https://github.com/$REPO"
    T_no_root="Isso precisa de root, e nem o sudo nem o doas me deixaram entrar, então pulei:"
    T_deps_fail="Não deu para instalar o CUPS (veja acima), então o PrinterTUI vai sem ele; a impressão funciona quando o CUPS estiver lá"
    T_own_pkgs="Neste sistema eu não instalo pacotes: adicione com as ferramentas dele os clientes do CUPS (lp, lpstat, lpq), o pdftoppm do poppler e os extras que você escolheu"
    T_path="A pasta do printertui ainda não está no seu PATH: adicione esta linha ao ~/.profile (ou ao arquivo de inicialização do seu shell) e abra um novo terminal"
    T_mdns="Para achar impressoras de rede pelo nome, coloque mdns_minimal [NOTFOUND=return] antes de resolve e dns na linha hosts do /etc/nsswitch.conf:"
}

lang_ru() {
    T_name="🇷🇺 Русский"
    T_hint="↑↓ выбор · enter ок · q выход"
    T_prev="(предпросмотр: ничего не устанавливается и не удаляется)"
    T_skip="предпросмотр, пропущено:"
    T_hi_new="Привет! Кажется, мы ещё не знакомы. Разогреть принтер?"
    T_hi_old="О, я уже установлен. Свежая копия или мне собирать вещи?"
    T_install="Установить" T_update="Обновить" T_uninstall="Удалить"
    T_deps_linux="Устанавливаю CUPS (sudo может спросить пароль)"
    T_cups="Запускаю CUPS"
    T_download="Скачиваю PrinterTUI"
    T_jam="Бумагу зажевало! Загрузка не удалась."
    T_put="Кладу printertui в"
    T_done="Страница напечатана, чернила высохли, всё готово. Запуск: printertui"
    T_rm_cmd="Удаляю команду printertui"
    T_rm_cfg="Удаляю настройки и временные файлы"
    T_bye="Бумага вернулась в лоток. Пока!"
    T_left="CUPS и ваши принтеры остались как были."
    T_quit="Ничего страшного, ничего не изменено."
    T_e_os="нужен Linux или macOS"
    T_e_brew="сначала нужен Homebrew: https://brew.sh"
    T_e_net="проверьте подключение к интернету и запустите установщик снова"
    T_extras="Немного дополнений? Всё по желанию."
    T_check_hint="↑↓ выбор · пробел отметить · enter дальше · q выход"
    T_lo="LibreOffice: печать Word, Excel и других файлов не в PDF"
    T_sane="SANE: USB-сканеры, которые не входят в принтер"
    T_ocr="Tesseract: текст с поиском в отсканированных PDF (OCR)"
    T_extras_inst="Устанавливаю дополнения"
    T_smudge="Страница смазана! С этой загрузкой что-то не так."
    T_e_sum="загрузка не совпадает с контрольной суммой, поэтому ничего не установлено"
    T_e_arch="для этого процессора нет готового PrinterTUI; соберите его сами: cargo install --git https://github.com/$REPO"
    T_e_404="для этой системы пока нет готового PrinterTUI (404); соберите его сами: cargo install --git https://github.com/$REPO"
    T_no_root="Здесь нужны права root, но ни sudo, ни doas меня не пустили, поэтому шаг пропущен:"
    T_deps_fail="CUPS установить не удалось (см. выше), поэтому PrinterTUI ставится без него; печать заработает, когда появится CUPS"
    T_own_pkgs="На этой системе я не ставлю пакеты: добавьте её собственными средствами клиентские утилиты CUPS (lp, lpstat, lpq), pdftoppm из poppler и выбранные дополнения"
    T_path="Папки printertui ещё нет в вашем PATH: добавьте эту строку в ~/.profile (или в файл запуска вашей оболочки) и откройте новый терминал"
    T_mdns="Чтобы находить сетевые принтеры по имени, добавьте mdns_minimal [NOTFOUND=return] перед resolve и dns в строке hosts файла /etc/nsswitch.conf:"
}

lang_id() {
    T_name="🇮🇩 Bahasa Indonesia"
    T_hint="↑↓ pindah · enter pilih · q keluar"
    T_prev="(pratinjau: tidak ada yang dipasang atau dihapus)"
    T_skip="pratinjau, dilewati:"
    T_hi_new="Halo! Sepertinya kita belum kenal. Panaskan printernya?"
    T_hi_old="Oh, aku sudah terpasang. Salinan baru, atau aku berkemas?"
    T_install="Pasang" T_update="Perbarui" T_uninstall="Copot"
    T_deps_linux="Memasang CUPS (sudo mungkin meminta kata sandi)"
    T_cups="Menyalakan CUPS"
    T_download="Mengunduh PrinterTUI"
    T_jam="Kertas macet! Unduhannya gagal."
    T_put="Menaruh printertui di"
    T_done="Halaman tercetak, tinta kering, siap dipakai. Jalankan: printertui"
    T_rm_cmd="Menghapus perintah printertui"
    T_rm_cfg="Menghapus pengaturan dan file sementara"
    T_bye="Kertas sudah kembali ke baki. Sampai jumpa!"
    T_left="CUPS dan printermu tetap seperti semula."
    T_quit="Tenang, tidak ada yang diubah."
    T_e_os="perlu Linux atau macOS"
    T_e_brew="perlu Homebrew dulu: https://brew.sh"
    T_e_net="periksa koneksi internet lalu jalankan installer lagi"
    T_extras="Mau tambahan? Semuanya opsional."
    T_check_hint="↑↓ pindah · spasi tandai · enter lanjut · q keluar"
    T_lo="LibreOffice: cetak Word, Excel, dan file non-PDF lainnya"
    T_sane="SANE: pemindai USB yang bukan bagian dari printer"
    T_ocr="Tesseract: teks yang bisa dicari di PDF hasil pindai (OCR)"
    T_extras_inst="Memasang tambahan"
    T_smudge="Halamannya belepotan! Ada yang aneh dengan unduhan itu."
    T_e_sum="unduhan tidak cocok dengan checksum-nya, jadi tidak ada yang dipasang"
    T_e_arch="belum ada PrinterTUI siap pakai untuk prosesor ini; bangun sendiri dengan: cargo install --git https://github.com/$REPO"
    T_e_404="belum ada PrinterTUI siap pakai untuk sistem ini (404); bangun sendiri dengan: cargo install --git https://github.com/$REPO"
    T_no_root="Ini butuh root, tapi sudo maupun doas tidak mengizinkanku, jadi kulewati:"
    T_deps_fail="CUPS tidak bisa dipasang (lihat di atas), jadi PrinterTUI dipasang tanpanya; mencetak bisa setelah CUPS ada"
    T_own_pkgs="Di sistem ini aku tidak memasang paket: tambahkan alat klien CUPS (lp, lpstat, lpq), pdftoppm dari poppler, dan tambahan yang kamu pilih dengan alat sistem itu sendiri"
    T_path="Folder printertui belum ada di PATH-mu: tambahkan baris ini ke ~/.profile (atau file awal shell-mu) lalu buka terminal baru"
    T_mdns="Agar printer jaringan bisa ditemukan lewat namanya, taruh mdns_minimal [NOTFOUND=return] sebelum resolve dan dns di baris hosts /etc/nsswitch.conf:"
}

# --- pixel art: two pixels per character with half blocks --------------------
# . empty  w paper  k ink  b blue  d dark body  g light body  s slot  G green  r red
PRINTER='......wwwwwwww......
......wwwwwwww......
..bbbbbbbbbbbbbbbb..
.bbbbbbbbbbbbbbbbbb.
.dggggggggggggggggd.
.dggggggggggggLgMgd.
.dggggggggggggggggd.
.dddddddddddddddddd.
..dssssssssssssssd..
...dddddddddddddd...'
PAPER='...wwwwwwwwwwwwww...
...wbbbbbbbbwwwww...
...wwwwwwwwwwwwww...
...wkkkkkkkkkkkkw...
...wkkkkkkkkwwwww...
...wwwwwwwwwwwwww...
...wkkkkkkkkkkkww...
...wkkkkkkwwwwwww...
...wwwwwwGGwwwwww...
...wwwwwwwwwwwwww...'
# crumpled page stuck in the slot, with a red smudge
JAM='...wwkwwwwwkwwww....
..w.wwwrrwwww.ww....
...ww.wwwkww.w.w....
....w.wwkw..w.......
......w..w..........'

# $1 columns of indent
render() {
    awk -v pad="$1" 'BEGIN { n = split("w 231 k 246 b 69 d 240 g 252 s 234 G 114 r 203", a, " ")
                 for (i = 1; i < n; i += 2) col[a[i]] = a[i + 1] }
         { row[NR] = $0 }
         END {
             for (r = 1; r <= NR; r += 2) {
                 o = ""
                 for (i = 0; i < pad; i++) o = o " "
                 for (i = 1; i <= length(row[r]); i++) {
                     t = substr(row[r], i, 1); u = substr(row[r + 1], i, 1)
                     if (t == "." && u == ".") o = o "\033[0m "
                     else if (t == ".") o = o "\033[0;38;5;" col[u] "m▄"
                     else if (u == ".") o = o "\033[0;38;5;" col[t] "m▀"
                     else o = o "\033[38;5;" col[t] ";48;5;" col[u] "m▀"
                 }
                 print o "\033[0m\033[K"
             }
         }'
}

# $1 rows of paper out of the slot (0-10), $2 $3 the two lights, $4 indent (2, shakes with 1 and 3)
frame() {
    {
        printf '%s\n' "$PRINTER" | sed "s/L/$2/; s/M/$3/"
        printf '%s\n' "${paper:-$PAPER}" | tail -n "$1"
        [ $(($1 % 2)) -eq 0 ] || echo ....................
    } | render "${4:-2}"
}

# redraws over the previous frame, so the art grows and shrinks with the paper
show() {
    [ -z "$drawn" ] || printf '\033[%dA' "$drawn"
    frame "$@"
    printf '\033[J'
    drawn=$(((11 + $1) / 2))
    sleep 0.09
}

blink() {
    if [ $(($1 % 2)) -eq 0 ]; then show "$1" G g; else show "$1" g G; fi
}

animate() {
    [ -n "$tty" ] || return 0
    printf '\033[?25l'
    # finish and jam carry on from where print stopped
    case $1 in finish | jam) ;; *) drawn= ;; esac
    case $1 in
        boot) # lights wake up one by one
            play boot
            for l in "g g" "G g" "g g" "G g" "G G"; do show 0 $l; done ;;
        print) # pages slide out while the lights blink, as long as process $2 runs, whirring
            n=0
            while kill -0 "$2" 2>/dev/null; do
                [ $n -ne 0 ] || play print
                blink $n
                n=$(((n + 1) % 11))
            done ;;
        finish) # the page that was halfway out comes all the way out
            while [ $n -le 10 ]; do
                blink $n
                n=$((n + 1))
            done
            play done
            show 10 G G ;;
        jam) # the page crumples in the slot, the printer shakes and flashes red
            play jam
            paper=$JAM
            i=0
            for x in 1 3 1 3 1 3 2 2 2 2; do
                if [ $((i % 2)) -eq 0 ]; then show 5 r r $x; else show 5 g g $x; fi
                i=$((i + 1))
            done
            show 5 r r
            paper= ;;
        unprint) # page gets pulled back in, lights go red
            play bye
            n=10
            while [ $n -ge 0 ]; do
                if [ $((n % 2)) -eq 0 ]; then show $n r g; else show $n g r; fi
                n=$((n - 1))
            done
            show 0 r r ;;
    esac
    printf '\033[?25h'
}

# --- menu: arrows or j/k, enter, q ------------------------------------------
readkey() {
    k=$(dd bs=1 count=1 2>/dev/null </dev/tty)
    case $k in
        "$esc")
            case $(dd bs=1 count=2 2>/dev/null </dev/tty) in
                '[A' | OA) k=up ;;
                '[B' | OB) k=down ;;
                *) k= ;;
            esac ;;
        '' | "$cr") k=enter ;;
    esac
}

# is option $1 marked in $checked (a string of 0 and 1, one per option)
marked() { [ "$(printf '%s' "$checked" | cut -c"$1")" = 1 ]; }

# starts on option $sel, leaves the number picked in sel (0 for quit) and only that option on screen.
# With $checked set it is a checklist: space marks, enter confirms, and the marked ones stay on screen.
menu() {
    hint=$T_hint
    [ -z "$checked" ] || hint=$T_check_hint
    stty -icanon -echo min 1 </dev/tty
    while :; do
        i=1
        for o; do
            box=
            if [ -n "$checked" ]; then
                if marked $i; then box='[x] '; else box='[ ] '; fi
            fi
            if [ $i -eq $sel ]; then
                printf '  \033[1;38;5;69m▶ %s%s\033[0m\033[K\n' "$box" "$o"
            else
                printf '    \033[2m%s%s\033[0m\033[K\n' "$box" "$o"
            fi
            i=$((i + 1))
        done
        printf '\n  \033[2m%s\033[0m\033[K\n' "$hint"
        readkey
        case $k in
            up | k) if [ $sel -gt 1 ]; then sel=$((sel - 1)); play blip; fi ;;
            down | j) if [ $sel -lt $# ]; then sel=$((sel + 1)); play blip; fi ;;
            ' ') if [ -n "$checked" ]; then
                     checked=$(printf '%s' "$checked" | awk -v i=$sel '{ print substr($0, 1, i - 1) (1 - substr($0, i, 1)) substr($0, i + 1) }')
                 fi ;;
            enter) break ;;
            q | Q) sel=0; break ;;
        esac
        printf '\033[%dA' $(($# + 2))
    done
    stty "$saved" </dev/tty
    printf '\033[%dA\033[J' $(($# + 2))
    [ $sel -gt 0 ] || return 0
    if [ -n "$checked" ]; then
        i=1
        for o; do
            if marked $i; then printf '  \033[1;38;5;114m✓ %s\033[0m\n' "$o"; fi
            i=$((i + 1))
        done
    else
        eval "picked=\${$sel}"
        printf '  \033[1;38;5;69m▶ %s\033[0m\n' "$picked"
    fi
}

# --- the actual work --------------------------------------------------------
# where this installer puts printertui, plus ~/.local/bin from older installs.
# A printertui found elsewhere on PATH (a development build) is not ours and is left alone.
bins() {
    [ -z "${PRINTERTUI_BIN_DIR:-}" ] || echo "${PRINTERTUI_BIN_DIR%/}/printertui"
    echo /usr/local/bin/printertui
    if command -v brew >/dev/null; then echo "$(brew --prefix)/bin/printertui"; fi
    echo "$HOME/.local/bin/printertui"
}

# Tesseract's code for the language picked: it reads English plus that one
ocr_code() {
    case $lang in
        zh) echo chi_sim ;; hi) echo hin ;; es) echo spa ;; ar) echo ara ;; fr) echo fra ;;
        bn) echo ben ;; pt) echo por ;; ru) echo rus ;; id) echo ind ;; *) echo eng ;;
    esac
}

# extras: 1 LibreOffice, 2 SANE, 3 Tesseract (with English and the language picked); the ones
# already installed start marked, and are not installed again
has_extra() {
    case $1 in
        1) command -v soffice >/dev/null || command -v libreoffice >/dev/null || [ -d /Applications/LibreOffice.app ] || {
            # where the app finds it too: LibreOffice's own packages in /opt, the Flatpak
            for d in /opt/libreoffice*/program/soffice /var/lib/flatpak/app/org.libreoffice.LibreOffice \
                "$HOME/.local/share/flatpak/app/org.libreoffice.LibreOffice"; do
                [ ! -e "$d" ] || return 0
            done
            return 1
        } ;;
        2) command -v scanimage >/dev/null ;;
        3) command -v tesseract >/dev/null && tesseract --list-langs 2>&1 | grep -qx eng &&
            tesseract --list-langs 2>&1 | grep -qx "$(ocr_code)" ;;
    esac
}

# Printing's tools are there already: CUPS's lp, lpstat and lpq (the queue), poppler's pdftoppm (the
# preview); where packages come from here ($pm) also CUPS's scheduler, which the client tools come
# without on Debian, Fedora and openSUSE, and on Arch Avahi with nss-mdns (printers on the network).
have_base() {
    for c in lp lpstat lpq pdftoppm; do
        command -v $c >/dev/null || return 1
    done
    [ -z "$pm" ] || [ -e /usr/sbin/cupsd ] || [ -e /usr/bin/cupsd ] || return 1
    [ "$pm" != pacman ] || { command -v avahi-daemon >/dev/null && [ -e /usr/lib/libnss_mdns_minimal.so.2 ]; }
}

# The packages a Linux package manager ($1: pacman, apt, dnf or zypper) installs. "base" is what
# printing needs: CUPS (with lpq for the queue), poppler's pdftoppm for the preview, and on Arch
# Avahi with nss-mdns for network printers. "extras" are the extras marked in $3 (as "101":
# 1 LibreOffice, 2 SANE, 3 Tesseract; all when not given), Tesseract reading English and the
# language $4 (its tesseract code: spa, chi_sim...).
packages() {
    marks=${3:-111} ocr=${4:-eng}
    case $2 in
        base)
            case $1 in
                pacman) echo cups cups-filters poppler avahi nss-mdns ;;
                apt) echo cups cups-client cups-bsd cups-filters poppler-utils ;;
                dnf) echo cups cups-client cups-filters poppler-utils ;;
                zypper) echo cups cups-client cups-filters poppler-tools ;;
            esac ;;
        extras)
            pkgs=
            if [ "$(printf '%s' "$marks" | cut -c1)" = 1 ]; then
                case $1 in
                    pacman) pkgs="$pkgs libreoffice-fresh" ;;
                    *) pkgs="$pkgs libreoffice-writer libreoffice-calc libreoffice-impress" ;;
                esac
            fi
            if [ "$(printf '%s' "$marks" | cut -c2)" = 1 ]; then
                case $1 in
                    pacman) pkgs="$pkgs sane sane-airscan" ;;
                    apt) pkgs="$pkgs sane-utils sane-airscan" ;;
                    dnf) pkgs="$pkgs sane-backends libsane-airscan" ;;
                    zypper) pkgs="$pkgs sane-backends sane-airscan" ;;
                esac
            fi
            if [ "$(printf '%s' "$marks" | cut -c3)" = 1 ]; then
                for l in $(printf 'eng %s' "$ocr" | tr ' ' '\n' | sort -u); do
                    case $1 in
                        pacman) pkgs="$pkgs tesseract-data-$l" ;;
                        apt) pkgs="$pkgs tesseract-ocr-$(printf '%s' "$l" | tr _ -)" ;;
                        dnf) pkgs="$pkgs tesseract-langpack-$l" ;;
                        zypper)
                            case $l in
                                eng) l=english ;; spa) l=spanish ;; fra) l=french ;; por) l=portuguese ;; rus) l=russian ;;
                                ara) l=arabic ;; hin) l=hindi ;; ben) l=bengali ;; ind) l=indonesian ;; chi_sim) l=chinese_simplified ;;
                            esac
                            pkgs="$pkgs tesseract-ocr-traineddata-$l" ;;
                    esac
                done
                case $1 in
                    pacman | dnf) pkgs="$pkgs tesseract" ;;
                    *) pkgs="$pkgs tesseract-ocr" ;;
                esac
            fi
            echo $pkgs ;;
    esac
}

# installs packages with the package manager in $pm
install_pkgs() {
    case $pm in
        pacman) as_root pacman -S --needed --noconfirm "$@" ;;
        apt)
            # a source that is down (an old PPA) fails the update, not the install
            as_root apt-get update -qq || true
            as_root apt-get install -y "$@" ;;
        dnf) as_root dnf install -y "$@" ;;
        zypper) as_root zypper --non-interactive install "$@" ;;
    esac
}

# Image-based systems, whose packages are not added from here: prints ostree (Silverblue, Kinoite,
# Bazzite), nixos, steamos or ro (a read-only root, as on openSUSE MicroOS), or nothing.
immutable() {
    if [ -e /run/ostree-booted ]; then
        echo ostree
    elif [ -e /etc/NIXOS ]; then
        echo nixos
    elif grep -Eqx 'ID="?steamos"?' /etc/os-release 2>/dev/null; then
        echo steamos
    elif read_only /; then
        echo ro
    fi
}

# The command that adds what is missing (CUPS's client tools, pdftoppm, the extras in $want) on the
# image-based system $sys, when it has one: rpm-ostree only takes packages its image does not have yet.
own_line() {
    cups= pdf=
    for c in lp lpstat lpq; do
        command -v $c >/dev/null || cups=1
    done
    command -v pdftoppm >/dev/null || pdf=1
    # shellcheck disable=SC2046
    case $sys in
        ostree) set -- rpm-ostree install ${cups:+cups-client} ${pdf:+poppler-utils} $(packages dnf extras "$want" "$(ocr_code)") ;;
        ro) command -v transactional-update >/dev/null || return 0
            set -- sudo transactional-update pkg install ${cups:+cups-client} ${pdf:+poppler-tools} $(packages zypper extras "$want" "$(ocr_code)") ;;
        nixos) set -- ${cups:+services.printing.enable = true;} ${pdf:+environment.systemPackages = [ pkgs.poppler_utils ];} ;;
        *) return 0 ;;
    esac
    case "$*" in *install) ;; *) echo "$*" ;; esac
}

# $1, or the nearest folder above it that exists
existing() {
    d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    echo "$d"
}

# folder $1 (or where it would be made) is on a read-only filesystem, which root cannot write either
read_only() { findmnt -no OPTIONS -T "$(existing "$1")" 2>/dev/null | tail -n 1 | tr , '\n' | grep -qx ro; }

# The hosts line of nsswitch.conf $1 with mdns_minimal before resolve or dns, as Avahi needs to find
# printers by their .local names; nothing when it has mdns already.
mdns_line() {
    grep '^hosts:' "$1" 2>/dev/null | head -n 1 | awk '/mdns/ { exit }
        { for (i = 2; i <= NF; i++) if ($i == "resolve" || $i == "dns") { $i = "mdns_minimal [NOTFOUND=return] " $i; print; exit } }'
}

# puts the downloaded printertui into folder $1 (made when missing), as root when it is not the user's
put() {
    if [ -w "$(existing "$1")" ]; then
        { [ -d "$1" ] || run mkdir -p "$1"; } && run install -m 755 "$tmp" "$1/printertui"
    else
        { [ -d "$1" ] || as_root mkdir -p "$1"; } && as_root install -m 755 "$tmp" "$1/printertui"
    fi
}

sha() {
    if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

# downloads $1 into $2: 0 ok, 3 not there (HTTP 404), 1 no connection or another failure
get() {
    code=$(curl -fsL -w '%{http_code}' "$1" -o "$2") && return 0
    [ "$code" != 404 ] || return 3
    return 1
}

# downloads the binary and its checksum into $tmp: 0 ok, 1 download failed, 2 checksum mismatch,
# 3 no such file (no release for this system)
fetch() {
    if [ -n "$preview" ]; then
        sleep 3
        case $preview in jam) return 1 ;; smudge) return 2 ;; esac
        return 0
    fi
    get "$url" "$tmp" && get "$url.sha256" "$tmp.sha256" || return $?
    [ "$(sha "$tmp")" = "$(cut -d' ' -f1 "$tmp.sha256")" ] || return 2
}

do_uninstall() {
    say "$T_rm_cmd"
    while IFS= read -r bin; do
        run rm -f "$bin" 2>/dev/null || as_root rm -f "$bin" || true
    done <<EOF
$(bins)
EOF
    say "$T_rm_cfg"
    # the app's temporary files: per user now (in the cache folder when /tmp has someone else's), and
    # the copies LibreOffice's snap or Flatpak reads
    run rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/printertui" "${XDG_CACHE_HOME:-$HOME/.cache}/printertui" \
        "${TMPDIR:-/tmp}/printertui-$(id -u)" "$HOME/snap/libreoffice/common/printertui" \
        "$HOME/.var/app/org.libreoffice.LibreOffice/cache/printertui"
    # in one folder for everyone before, which may be another user's
    run rm -rf "${TMPDIR:-/tmp}/printertui" "${TMPDIR:-/tmp}/printertui-scan" "${TMPDIR:-/tmp}/printertui-sounds" 2>/dev/null || true
    # installed from source by an earlier run
    if [ -f "$HOME/.local/.crates.toml" ] && command -v cargo >/dev/null; then
        run cargo uninstall --root "$HOME/.local" printertui 2>/dev/null || true
    fi
}

do_install() {
    # only these have a release, so this comes before anything is installed
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        *) die "$(uname -m): $T_e_arch" ;;
    esac
    # the extras marked that are not there yet: an update with nothing new installs nothing
    checked=$extras want=
    for e in 1 2 3; do
        if marked $e && ! has_extra $e; then want=${want}1; else want=${want}0; fi
    done
    checked=

    case "$(uname -s)" in
        Linux)
            # image-based systems add packages their own way, other package managers are not known here:
            # there PrinterTUI goes in on its own, with a hint for the packages
            sys=$(immutable) pm=
            if [ -z "$sys" ]; then
                for p in pacman apt-get dnf zypper; do
                    if command -v $p >/dev/null; then pm=${p%-get}; break; fi
                done
            fi
            if [ -z "$pm" ]; then
                if ! have_base || [ "$want" != 000 ]; then warn "$T_own_pkgs" "$(own_line)"; fi
            else
                if ! have_base; then
                    say "$T_deps_linux"
                    # a package that cannot be installed (a mirror down, no root) does not stop PrinterTUI
                    # shellcheck disable=SC2046
                    install_pkgs $(packages $pm base) || warn "$T_deps_fail"
                fi
                # Debian and Ubuntu start CUPS themselves; WSL and containers may have no systemd.
                # On Arch, Avahi finds the printers on the network.
                if command -v systemctl >/dev/null; then
                    units=cups.socket todo=
                    [ $pm != pacman ] || units="$units avahi-daemon.service"
                    for u in $units; do
                        systemctl is-enabled --quiet $u 2>/dev/null || todo="$todo $u"
                    done
                    [ -z "$todo" ] || say "$T_cups"
                    for u in $todo; do
                        as_root systemctl enable --now $u || true
                    done
                fi
                if [ "$want" != 000 ]; then
                    say "$T_extras_inst"
                    # an extra the system cannot install (its package manager says why) does not stop PrinterTUI
                    # shellcheck disable=SC2046
                    install_pkgs $(packages $pm extras "$want" "$(ocr_code)") || true
                fi
                # nsswitch.conf is the system's own: say what to change, not change it
                if [ $pm = pacman ] && line=$(mdns_line /etc/nsswitch.conf) && [ -n "$line" ]; then
                    warn "$T_mdns" "$line"
                fi
            fi
            # already on every PATH, so printertui runs right away in this terminal
            bin_dir=/usr/local/bin
            os=linux ;;
        Darwin)
            # PrinterTUI itself needs nothing: macOS has CUPS, PDFKit and Vision; only the extras come from Homebrew
            if [ "$want" != 000 ] && ! command -v brew >/dev/null; then
                say "$T_e_brew"
            elif [ "$want" != 000 ]; then
                say "$T_extras_inst"
                checked=$want
                if marked 1; then run brew install --cask libreoffice || true; fi
                if marked 2; then run brew install sane-backends || true; fi
                checked=
            fi
            # Homebrew's bin is the user's own; /usr/local/bin is on every macOS PATH
            if command -v brew >/dev/null; then bin_dir="$(brew --prefix)/bin"; else bin_dir=/usr/local/bin; fi
            os=macos ;;
        *) die "$T_e_os" ;;
    esac

    # printertui's folder: PRINTERTUI_BIN_DIR, else where it is already (an update), else the one above
    # while it can be written (as root too), else the user's own
    if [ -n "${PRINTERTUI_BIN_DIR:-}" ]; then
        bin_dir=${PRINTERTUI_BIN_DIR%/}
    else
        while IFS= read -r bin; do
            if [ -e "$bin" ]; then
                bin_dir=${bin%/*}
                break
            fi
        done <<EOF
$(bins)
EOF
        if [ ! -w "$(existing "$bin_dir")" ] && { ! rootable || read_only "$bin_dir"; }; then bin_dir=$HOME/.local/bin; fi
    fi

    tmp=$(mktemp)
    url="https://github.com/$REPO/releases/latest/download/printertui-$os-$arch"
    say "$T_download ($os $arch)"
    [ -z "$preview" ] || printf '    \033[2m%s curl %s %s.sha256\033[0m\n' "$T_skip" "$url" "$url"
    status=0
    if [ -n "$tty" ]; then
        # the printer prints while the download runs in the background
        echo
        fetch &
        pid=$!
        animate print $pid
        wait $pid || status=$?
        if [ $status -eq 0 ]; then animate finish; else animate jam; fi
    else
        fetch || status=$?
    fi
    case $status in
        0) echo ;;
        2) talk "$T_smudge" pet 203; die "$T_e_sum" ;;
        3) talk "$T_jam" pet 203; die "$T_e_404" ;;
        *) talk "$T_jam" pet 203; die "$T_e_net" ;;
    esac

    say "$T_put $bin_dir"
    # (Apple silicon Macs may not have /usr/local/bin yet.) When root is refused after all, the user's own
    # folder, quietly: the skipped command names the download, which is gone when this ends.
    [ "$bin_dir" = "$HOME/.local/bin" ] || quiet=1
    if ! put "$bin_dir"; then
        quiet=
        [ "$bin_dir" != "$HOME/.local/bin" ] || exit 1
        bin_dir=$HOME/.local/bin
        say "$T_put $bin_dir"
        put "$bin_dir"
    fi
    quiet=
    # an older install in ~/.local/bin would shadow this one (one that stays does not undo the install)
    if [ "$(cd "$bin_dir" 2>/dev/null && pwd -P)" != "$(cd "$HOME/.local/bin" 2>/dev/null && pwd -P)" ]; then
        run rm -f "$HOME/.local/bin/printertui" 2>/dev/null || as_root rm -f "$HOME/.local/bin/printertui" || true
    fi
    # a folder that is not on PATH (~/.local/bin on some systems): the line that puts it there
    case ":$PATH:" in
        *":$bin_dir:"* | *":$bin_dir/:"*) ;;
        *)
            case $bin_dir in "$HOME"/*) shown="\$HOME/${bin_dir#"$HOME"/}" ;; *) shown=$bin_dir ;; esac
            warn "$T_path" "export PATH=\"$shown:\$PATH\"" ;;
    esac
}

# the app speaks the language picked in the menu (without a menu it follows the system's)
save_lang() {
    [ -n "$lang_picked" ] || return 0
    conf="${XDG_CONFIG_HOME:-$HOME/.config}/printertui/config"
    if [ -n "$preview" ]; then
        printf '    \033[2m%s lang=%s > %s\033[0m\n' "$T_skip" "$lang" "$conf"
        return
    fi
    mkdir -p "$(dirname "$conf")"
    { grep -v '^lang=' "$conf" 2>/dev/null; echo "lang=$lang"; } > "$conf.tmp" && mv "$conf.tmp" "$conf"
}

# --- main -------------------------------------------------------------------
installed=
if [ -n "$preview" ]; then
    if [ "$preview" = installed ]; then installed=1; fi
else
    while IFS= read -r bin; do
        if [ -e "$bin" ]; then installed=1; fi
    done <<EOF
$(bins)
EOF
fi
extras=000 checked=

# the system language is preselected in the menu, and used as is when there is no menu
sel=1 lang=en lang_picked=
for c in $LANGS; do
    case ${LC_ALL:-${LC_MESSAGES:-${LANG:-}}} in "$c"*) lang=$c ;; esac
done
# the system's language comes first in the menu
order="$lang $(printf '%s\n' $LANGS | grep -vx "$lang" | tr '\n' ' ')"
lang_$lang

action=${1:-}
if [ -z "$action" ] && [ -n "$tty" ]; then
    printf '\n  \033[1mPrinterTUI\033[0m\n\n'
    set --
    for c in $order; do lang_$c; set -- "$@" "$T_name"; done
    lang_$lang
    menu "$@"
    if [ $sel -eq 0 ]; then talk "$T_quit"; exit 0; fi
    # shellcheck disable=SC2086
    set -- $order
    eval "lang=\${$sel}"
    lang_$lang
    lang_picked=1
    [ -z "$preview" ] || printf '\n  \033[33m%s\033[0m\n' "$T_prev"
    echo
    animate boot
    sel=1
    if [ -n "$installed" ]; then
        talk "$T_hi_old" pet
        menu "$T_update" "$T_uninstall"
        set -- quit update uninstall
    else
        talk "$T_hi_new" pet
        menu "$T_install"
        set -- quit install
    fi
    eval "action=\${$((sel + 1))}"
    if [ "$action" != uninstall ] && [ "$action" != quit ]; then
        talk "$T_extras"
        sel=1 checked=
        for e in 1 2 3; do
            if has_extra $e; then checked=${checked}1; else checked=${checked}0; fi
        done
        if [ "$(uname -s)" = Darwin ]; then
            # macOS reads text in scans itself (Vision): no Tesseract to offer
            checked=${checked%?}
            menu "$T_lo" "$T_sane"
            checked=${checked}0
        else
            menu "$T_lo" "$T_sane" "$T_ocr"
        fi
        if [ $sel -eq 0 ]; then action=quit; else extras=$checked; fi
        checked=
    fi
    echo
fi

case ${action:-install} in
    install | update | reinstall)
        do_install
        save_lang
        talk "$T_done" ;;
    uninstall)
        do_uninstall
        echo
        animate unprint
        talk "$T_bye" pet
        say "$T_left" ;;
    quit)
        talk "$T_quit" ;;
    *) die "unknown option: $action (install, update or uninstall)" ;;
esac
