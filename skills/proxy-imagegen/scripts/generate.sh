#!/usr/bin/env bash
# Generate through the active Codex provider; runtime: Bash, curl, jq and coreutils.
set +x
set -euo pipefail
umask 077

fail() { printf 'proxy-imagegen: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Usage: generate.sh --prompt TEXT [--out FILE] [--attempts 1..10]
                   [--config FILE] [--auth FILE] [--timeout SECONDS]
Uses the active Codex provider's Responses API and GPT Image 2.
Maximum attempts includes the first (default: 3). No Python is required.
HELP
}
prompt=''; attempts=3; timeout=600
config="${CODEX_HOME:-$HOME/.codex}/config.toml"
auth="${CODEX_HOME:-$HOME/.codex}/auth.json"
output="${CODEX_HOME:-$HOME/.codex}/generated_images/proxy-imagegen.png"
while (($#)); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --prompt|--out|--attempts|--config|--auth|--timeout)
            (($# >= 2)) || fail "Missing value for $1"
            case "$1" in
                --prompt) prompt=$2 ;; --out) output=$2 ;; --attempts) attempts=$2 ;;
                --config) config=$2 ;; --auth) auth=$2 ;; --timeout) timeout=$2 ;;
            esac
            shift 2 ;;
        *) fail 'Unknown argument; see --help' ;;
    esac
done
[[ -n $prompt ]] || fail '--prompt is required'
[[ $attempts =~ ^([1-9]|10)$ ]] || fail '--attempts must be in 1..10'
[[ $timeout =~ ^[1-9][0-9]*$ ]] || fail '--timeout must be positive seconds'
for dependency in curl jq awk base64 od mktemp; do
    command -v "$dependency" >/dev/null || fail "Missing dependency: $dependency"
done
expand_home() { case "$1" in \~/*) printf '%s/%s' "$HOME" "${1:2}" ;; *) printf '%s' "$1" ;; esac; }
config=$(expand_home "$config"); auth=$(expand_home "$auth"); output=$(expand_home "$output")
[[ -r $config && -r $auth ]] || fail 'Codex config.toml or auth.json is unreadable'
work=$(mktemp -d)
curl_pid=''; writer_pid=''
trap '
    if [[ -n $curl_pid ]]; then kill "$curl_pid" 2>/dev/null || true; wait "$curl_pid" 2>/dev/null || true; fi
    if [[ -n $writer_pid ]]; then kill "$writer_pid" 2>/dev/null || true; wait "$writer_pid" 2>/dev/null || true; fi
    rm -rf -- "$work"
' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Read only required one-line TOML strings. Never source/eval config contents.
# Other settings are ignored; unsupported syntax in required values fails closed.
if ! awk '
function uncomment(s,    i,c,q,escaped,out) {
    q=""; escaped=0; out=""
    for(i=1;i<=length(s);i++) {
        c=substr(s,i,1)
        if(q=="" && c=="#") break
        out=out c
        if(escaped) { escaped=0; continue }
        if(q=="\"" && c=="\\") { escaped=1; continue }
        if(q!="" && c==q) q=""
        else if(q=="" && (c=="\"" || c==sprintf("%c",39))) q=c
    }
    sub(/^[ \t]+/,"",out); sub(/[ \t\r]+$/,"",out); return out
}
{
    line=uncomment($0)
    if(line ~ /^\[/) {
        scope=line; sub(/^\[[ \t]*/,"",scope); sub(/[ \t]*\]$/,"",scope); next
    }
    eq=index(line,"="); if(!eq) next
    key=substr(line,1,eq-1); gsub(/[ \t]/,"",key)
    if((scope=="" && (key=="model" || key=="model_provider")) || key=="base_url")
        print scope "\t" key "\t" uncomment(substr(line,eq+1))
}' "$config" | jq -Rn '
    def trim: gsub("^\\s+|\\s+$"; "");
    def toml_string:
        trim | if startswith("\"") then fromjson
        elif startswith("\u0027") and endswith("\u0027") then .[1:-1]
        else error("Required TOML value must be a one-line quoted string") end
        | if type == "string" and length > 0 then . else error("Empty TOML string") end;
    def provider_section:
        if startswith("model_providers.") then
            ltrimstr("model_providers.") | trim |
            if test("^[A-Za-z0-9_-]+$") then . else toml_string end
        else empty end;
    [inputs | capture("^(?<scope>[^\\t]*)\\t(?<key>[^\\t]+)\\t(?<raw>.*)$")] as $rows |
    def root($key): [$rows[] | select(.scope == "" and .key == $key) | .raw | toml_string] |
        if length == 1 then .[0] else error("Missing or duplicate root setting") end;
    root("model_provider") as $provider | root("model") as $model |
    [$rows[] | select(.key == "base_url") |
        select((.scope | provider_section) == $provider) | .raw | toml_string] |
    if length != 1 then error("Missing or duplicate active provider URL") else
        {model: $model, base_url: (.[0] | sub("/+$"; ""))} end
' >"$work/settings.json" 2>"$work/config-error"; then
    fail 'Cannot read model, model_provider and active base_url as one-line TOML strings'
fi
jq -e '.base_url | test("^https?://[^\\r\\n]+$")' "$work/settings.json" >/dev/null || fail 'Invalid provider URL'
# jq reads the key from auth.json itself: it never appears in process arguments.
if ! jq -er '
    .OPENAI_API_KEY | select(type == "string" and length > 0 and (test("[\\r\\n]") | not)) |
    "header = " + (("Authorization: Bearer " + .) | @json)
' "$auth" >"$work/curl.conf" 2>/dev/null; then
    fail 'auth.json has no valid OPENAI_API_KEY'
fi
jq -r '"url = " + ((.base_url + "/responses") | @json)' "$work/settings.json" >>"$work/curl.conf"
printf '%s' "$prompt" >"$work/prompt"
jq --rawfile prompt "$work/prompt" '{model, input:$prompt,
    tools:[{type:"image_generation",model:"gpt-image-2"}],
    tool_choice:{type:"image_generation"},stream:true,store:false}' "$work/settings.json" >"$work/request.json"

handle_event() {
    [[ -n $event_data && $event_data != '[DONE]' ]] || return 0
    printf '%s' "$event_data" >"$work/event.json"
    local kind
    kind=$(jq -r 'if type == "object" then .type // "" else "" end | select(type == "string")' "$work/event.json" 2>/dev/null) || return 0
    case "$kind" in
        response.output_item.done)
            if jq -er '.item | select(.type == "image_generation_call") | .result | select(type == "string" and length > 0)' \
                "$work/event.json" >"$work/candidate" 2>/dev/null; then
                mv "$work/candidate" "$work/image.b64"
            fi ;;
        response.completed)
            if jq -er '[.response.output[]? | select(.type == "image_generation_call") | .result |
                    select(type == "string" and length > 0)][0] // empty' \
                    "$work/event.json" >"$work/candidate" 2>/dev/null; then
                mv "$work/candidate" "$work/image.b64"
            fi
            completed=1 ;;
        response.failed|error) stream_failed=1 ;;
    esac
}

for ((attempt=1; attempt<=attempts; attempt++)); do
    rm -f "$work/image.b64" "$work/stream" "$work/headers" "$work/curl.pid"
    mkfifo "$work/stream"
    completed=0; stream_failed=0; event_data=''
    # The writer opens the FIFO before invoking curl, so every transport failure
    # still closes the stream. No credential-bearing arguments or stderr escape.
    (
        exec 3>"$work/stream"
        curl --config "$work/curl.conf" --silent --no-buffer --connect-timeout 30 \
            --max-time "$timeout" --header 'Content-Type: application/json' \
            --header 'Accept: text/event-stream' --data-binary "@$work/request.json" \
            --dump-header "$work/headers" >&3 2>"$work/transport-error" &
        writer_curl=$!
        trap 'kill "$writer_curl" 2>/dev/null || true; wait "$writer_curl" 2>/dev/null || true' EXIT
        printf '%s' "$writer_curl" >"$work/curl.pid"
        wait "$writer_curl"
    ) &
    writer_pid=$!
    while IFS= read -r line || [[ -n $line ]]; do
        line=${line%$'\r'}
        if [[ -z $line ]]; then
            handle_event
            event_data=''
            if ((completed || stream_failed)); then break; fi
        elif [[ $line == data:* ]]; then
            part=${line#data:}; part=${part# }
            if [[ -n $event_data ]]; then event_data+=$'\n'; fi
            event_data+=$part
        fi
    done <"$work/stream"
    if [[ -n $event_data ]] && ((!completed && !stream_failed)); then handle_event; fi
    if [[ -f $work/curl.pid ]]; then curl_pid=$(cat "$work/curl.pid"); fi
    if [[ -n $curl_pid ]]; then kill "$curl_pid" 2>/dev/null || true; fi
    wait "$writer_pid" 2>/dev/null || true
    curl_pid=''; writer_pid=''
    status=$(awk '/^HTTP\// {code=$2} END {print code}' "$work/headers" 2>/dev/null || true)
    reason='Connection failed or SSE ended before response.completed'
    if [[ $status =~ ^[45][0-9][0-9]$ ]]; then
        reason="Responses API HTTP $status"
        if [[ $status != 408 && $status != 429 && $status != 5* ]]; then fail "$reason"; fi
    elif [[ $status =~ ^2[0-9][0-9]$ ]] && ((completed && !stream_failed)); then
        if [[ -s $work/image.b64 ]]; then
            # Accept a data URI as well as plain Base64; reject other characters.
            sed 's/^data:[^,]*,//' "$work/image.b64" | tr -d '\n' >"$work/encoded"
            reason='Invalid Base64 or PNG/JPEG image result'
            if ! LC_ALL=C grep -q '[^A-Za-z0-9+/=]' "$work/encoded" &&
                base64 --decode "$work/encoded" >"$work/image" 2>/dev/null && [[ -s $work/image ]]; then
                signature=$(od -An -tx1 -N8 "$work/image" | tr -d ' \n')
                if [[ $signature == 89504e470d0a1a0a || $signature == ffd8ff* ]]; then
                    mkdir -p -- "$(dirname -- "$output")"
                    output="$(cd -- "$(dirname -- "$output")" && pwd -P)/$(basename -- "$output")"
                    # Same-directory rename preserves any previous output on failure.
                    destination=$(mktemp "$(dirname -- "$output")/.proxy-imagegen.XXXXXX")
                    if ! cp -- "$work/image" "$destination" || ! mv -f -- "$destination" "$output"; then
                        rm -f -- "$destination"; fail 'Cannot save output image'
                    fi
                    printf '%s\nbytes=%s\n' "$output" "$(wc -c <"$output" | tr -d ' ')"
                    exit 0
                fi
            fi
        else reason='response.completed contains no image result'; fi
    elif ((stream_failed)); then reason='SSE response.failed or error'; fi
    printf 'proxy-imagegen: attempt %s/%s: %s\n' "$attempt" "$attempts" "$reason" >&2
    if ((attempt < attempts)); then
        delay=$((2 ** attempt)); ((delay <= 10)) || delay=10
        sleep "$delay"
    fi
done
exit 1
