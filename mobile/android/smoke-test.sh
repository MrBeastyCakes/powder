#!/usr/bin/env bash
# A rendered frame and a surviving process are both required. Installing an APK
# or starting a NativeActivity successfully does not prove native startup works.
set -euo pipefail

apk=${1:?Usage: smoke-test.sh APK [RESULTS_DIRECTORY]}
results=${2:-android-smoke-results}
package=app.powdertoy.next
activity="$package/android.app.NativeActivity"
frame_marker='startup: first frame presented'
logcat_pid=
mkdir -p "$results"

adb_command() {
    timeout 30s adb "$@"
}

screenshot() {
    adb_command exec-out screencap -p > "$results/$1.png"
}

cleanup() {
    result=$?
    trap - EXIT
    set +e
    screenshot final
    adb_command logcat -b crash -d -v threadtime > "$results/crash.txt"
    adb_command shell dumpsys activity activities > "$results/activities.txt"
    adb_command shell dumpsys window > "$results/window.txt"
    if [[ -n "$logcat_pid" ]]; then
        kill "$logcat_pid" 2>/dev/null
        wait "$logcat_pid" 2>/dev/null
    fi
    if (( result != 0 )); then
        echo "Android startup smoke failed; last native/system messages follow:" >&2
        tail -n 120 "$results/logcat.txt" >&2
    fi
    exit "$result"
}
trap cleanup EXIT

app_pid() {
    adb_command shell pidof "$package" 2>/dev/null | tr -d '\r'
}

require_alive() {
    local actual
    actual=$(app_pid) || true
    if [[ -z "$actual" || "$actual" != "$expected_pid" ]]; then
        echo "$phase: app exited or restarted (expected PID $expected_pid, found ${actual:-none})" >&2
        return 1
    fi
}

wait_for_frame() {
    local deadline=$((SECONDS + 90))
    local seen_pid=
    local current_pid
    while (( SECONDS < deadline )); do
        current_pid=$(app_pid) || true
        if [[ -n "$seen_pid" && "$current_pid" != "$seen_pid" ]]; then
            echo "$phase: native process exited before presenting its first frame" >&2
            return 1
        fi
        if [[ -n "$current_pid" ]]; then
            seen_pid=$current_pid
        fi
        if tail -n "+$phase_log_line" "$results/logcat.txt" | grep -F "$frame_marker" > /dev/null; then
            expected_pid=$current_pid
            require_alive
            return
        fi
        sleep 1
    done
    echo "$phase: no presented frame within 90 seconds" >&2
    return 1
}

check_launch() {
    phase=$1
    phase_log_line=$(( $(wc -l < "$results/logcat.txt") + 1 ))
    echo "Checking $phase"
    # am start can report success even when the native renderer later panics.
    timeout 90s adb shell am start -W -n "$activity" > "$results/$phase-launch.txt"
    wait_for_frame
    # Catch crashes after first presentation as well as during initialization.
    for ((second = 0; second < 5; second++)); do
        sleep 1
        require_alive
    done
    screenshot "$phase"
}

adb_command wait-for-device
adb_command shell getprop > "$results/device-properties.txt"
adb_command shell pm list features > "$results/device-features.txt"
adb_command install -r "$apk" > "$results/install.txt"
adb_command shell am force-stop "$package"
adb_command logcat -c
adb logcat -v threadtime > "$results/logcat.txt" 2>&1 &
logcat_pid=$!

check_launch cold-start
adb_command shell input keyevent KEYCODE_HOME
sleep 2
require_alive
check_launch resume
adb_command shell am force-stop "$package"
check_launch relaunch
echo "Android startup smoke passed: three rendered launches, with background/resume and no native process exits."
