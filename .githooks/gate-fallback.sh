#!/bin/sh
# gate fallback: 没有 pengj-templates-cli 的机器上的提交信息检查.
#
# 权威始终是 CLI：`.gate.toml` 里的完整规则由它实现。本脚本只拦最容易犯的那几类错，让没装 CLI 的机器也不至于把离谱的提交放过去。这里有一条刻意守住的不变式：本脚本永远比 CLI 宽松——可能放过坏消息，绝不误杀好消息，所以最终裁决仍在服务端 CI。
#
# 只依赖 git 与一个 POSIX shell：没有构建步骤，也没有依赖要装。阈值从 `.gate.toml` 读，项目改过配置这里也跟着改。

set -u

fail() {
    printf '[gate] %s\n' "$1" >&2
    if [ -n "${2:-}" ]; then
        printf '  %s\n' "$2" >&2
    fi
    printf '[gate] %s\n' "没装 pengj-templates-cli，这里跑的是缩减规则集，完整规则集由服务端 CI 把关" >&2
    exit 1
}

# 文案集中放在这里，下面的逻辑读起来就是规则本身。
M_HEADER="标题必须形如 type(scope): 摘要"
M_TRIM="标题首尾不能有空白字符"
M_TYPE_EMPTY="type 不能为空"
M_TYPE_ENUM="type 必须是允许的类型之一"
M_SUBJECT_EMPTY="摘要不能为空"
M_FULL_STOP="摘要不能以句号结尾"
M_LANG_ZH="摘要必须用中文"
M_LANG_EN="摘要必须用英文"

msg_file=${1:-}
if [ -z "$msg_file" ]; then
    fail "没有拿到提交信息文件" \
        "commit-msg 钩子会把提交信息文件作为第一个参数传进来"
fi
if [ ! -f "$msg_file" ]; then
    fail "提交信息文件不存在" "$msg_file"
fi

root=$(git rev-parse --show-toplevel 2>/dev/null) || root=""
# 钩子是在仓库里跑的，相对路径是常态；绝对路径只在脚本被从别处调用时才用得上。
config=".gate.toml"
if [ -n "$root" ] && [ -f "$root/.gate.toml" ]; then
    config="$root/.gate.toml"
fi

# 从 `.gate.toml` 读一个扁平键。该文件是扁平 TOML 子集，`key = value` 与 `section.key = value` 都落在同一行 `key = value` 上。
config_value() {
    [ -f "$config" ] || return 0
    key=$(printf '%s' "$1" | sed 's/[.]/\\./g')
    sed -n "s/^[[:space:]]*$key[[:space:]]*=[[:space:]]*\\(.*\\)$/\\1/p" "$config" | head -n 1
}

# `.gate.toml` 里可以用 `disabled = ["commit"]` 关掉某项检查。CLI 认这个开关，兜底脚本也得认。
disabled=$(config_value disabled | tr -d '[]"' | tr ',' ' ')
project_disabled=$(config_value project.disabled | tr -d '[]"' | tr ',' ' ')
case " $disabled $project_disabled " in
    *" commit "*) exit 0 ;;
esac

types=$(config_value commit.types | tr -d '[]"' | tr ',' ' ')
lang=$(config_value commit.subject_lang | tr -d '"')
min_han=$(config_value commit.subject_min_han)
case "$min_han" in '' | *[!0-9]*) min_han=2 ;; esac

# 第一条有效行就是标题。注释行与空行会被跳过，与 CLI 的处理一致。
header=$(awk 'NF && $0 !~ /^[[:space:]]*#/ { print; exit }' "$msg_file" 2>/dev/null)
if [ -z "$header" ]; then
    fail "提交信息是空的"
fi

# 合并提交、revert、fixup、amend 一律放行，与 CLI 的豁免一致。
case "$header" in
    Merge\ * | Revert\ * | revert:* | fixup!* | squash!* | amend!*) exit 0 ;;
esac

case "$header" in
    " "* | *" ") fail "$M_TRIM" ;;
esac

case "$header" in
    *": "*) ;;
    *) fail "$M_HEADER" "例如：feat(cli): 新增一个参数" ;;
esac

prefix=${header%%: *}
subject=${header#*: *}

# 把 `type(scope)!` 拆成各部分。
scope=""
case "$prefix" in
    *\(*\)*)
        type=${prefix%%(*}
        scope=${prefix#*(}
        scope=${scope%%)*}
        ;;
    *)
        type=${prefix%!}
        ;;
esac

if [ -z "$type" ]; then
    fail "$M_TYPE_EMPTY" "$M_HEADER"
fi
if [ -n "$types" ]; then
    case " $types " in
        *" $type "*) ;;
        *) fail "$M_TYPE_ENUM" "允许的类型： $types" ;;
    esac
fi

if [ -z "$subject" ]; then
    fail "$M_SUBJECT_EMPTY" "$M_HEADER"
fi

# 结尾是省略号可以，单句点不行。
case "$header" in
    *...) ;;
    *.) fail "$M_FULL_STOP" ;;
esac

# 按 UTF-8 序列的首字节数汉字：E4..E9 覆盖常用汉字，E3 与 EF 覆盖更罕见的扩展区。宽范围是故意多算的，好让中文规则保持宽松。
han_wide=$(printf '%s' "$subject" | LC_ALL=C tr -cd '\343-\351\357' | wc -c | tr -d ' ')

# 英文规则会忽略引号里的字面量，所以引号里引用的术语不算违规。
stripped=$(printf '%s' "$subject" | sed "s/'[^']*'//g; s/\"[^\"]*\"//g")
han_stripped=$(printf '%s' "$stripped" | LC_ALL=C tr -cd '\344-\351' | wc -c | tr -d ' ')
first_byte=$(printf '%s' "$stripped" | LC_ALL=C od -An -tx1 2>/dev/null | tr -d ' \n' | cut -c1-2)
starts_han=0
case "$first_byte" in
    e4 | e5 | e6 | e7 | e8 | e9) starts_han=1 ;;
esac

# revert 与 `main` scope 豁免语言规则，与 CLI 一致。
if [ "$scope" = "main" ]; then
    exit 0
fi

if [ "$lang" = "en" ]; then
    if [ "$han_stripped" -gt 0 ]; then
        # 不以汉字开头的主题可以带少量汉字，与 CLI 的宽容一致。
        if [ "$starts_han" -eq 0 ] && [ "$han_stripped" -le 4 ]; then
            exit 0
        fi
        fail "$M_LANG_EN" "必须引用某个术语时，把它放进引号里，例如 'some-term'"
    fi
else
    if [ "$han_wide" -lt "$min_han" ]; then
        fail "$M_LANG_ZH" "至少要 $min_han 个汉字，例如：feat(cli): 新增一个参数"
    fi
fi

exit 0