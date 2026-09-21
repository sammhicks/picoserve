#!/usr/bin/env bash

set -eo pipefail

cd "$(dirname "$0")"

TEXT_INCREASE_COUNT=0
TEXT_DECREASE_COUNT=0
TEXT_INCREASE_MIN=
TEXT_INCREASE_MAX=
TEXT_DECREASE_MIN=
TEXT_DECREASE_MAX=
DATA_INCREASE_COUNT=0
DATA_DECREASE_COUNT=0
DATA_INCREASE_MIN=
DATA_INCREASE_MAX=
DATA_DECREASE_MIN=
DATA_DECREASE_MAX=
BSS_INCREASE_COUNT=0
BSS_DECREASE_COUNT=0
BSS_INCREASE_MIN=
BSS_INCREASE_MAX=
BSS_DECREASE_MIN=
BSS_DECREASE_MAX=

while IFS=$'\t' read -r EXAMPLE MANIFEST EXPECTED_TEXT EXPECTED_DATA EXPECTED_BSS
do
    echo "$EXAMPLE"

    cargo clean --release --target thumbv6m-none-eabi --package "$EXAMPLE"

    cargo build --release --bin "$EXAMPLE"

    read -r ACTUAL_TEXT ACTUAL_DATA ACTUAL_BSS < <(
        size -B "target/thumbv6m-none-eabi/release/$EXAMPLE" |
            tail -n 1 |
            awk '{ print $1, $2, $3 }'
    )

    if [ "$ACTUAL_TEXT" -gt "$EXPECTED_TEXT" ]; then
        CHANGE=$((ACTUAL_TEXT - EXPECTED_TEXT))
        echo "  text: $EXPECTED_TEXT => $ACTUAL_TEXT (INCREASE of $CHANGE bytes)"
        ((++TEXT_INCREASE_COUNT))

        if [ -z "$TEXT_INCREASE_MIN" ] || [ "$CHANGE" -lt "$TEXT_INCREASE_MIN" ]; then
            TEXT_INCREASE_MIN=$CHANGE
        fi
        if [ -z "$TEXT_INCREASE_MAX" ] || [ "$CHANGE" -gt "$TEXT_INCREASE_MAX" ]; then
            TEXT_INCREASE_MAX=$CHANGE
        fi
    elif [ "$ACTUAL_TEXT" -lt "$EXPECTED_TEXT" ]; then
        CHANGE=$((EXPECTED_TEXT - ACTUAL_TEXT))
        echo "  text: $EXPECTED_TEXT => $ACTUAL_TEXT (DECREASE of $CHANGE bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_text[[:space:]]*=[[:space:]]*[0-9]+/expected_text = $ACTUAL_TEXT/
            }" \
            "$MANIFEST"
        ((++TEXT_DECREASE_COUNT))

        if [ -z "$TEXT_DECREASE_MIN" ] || [ "$CHANGE" -lt "$TEXT_DECREASE_MIN" ]; then
            TEXT_DECREASE_MIN=$CHANGE
        fi
        if [ -z "$TEXT_DECREASE_MAX" ] || [ "$CHANGE" -gt "$TEXT_DECREASE_MAX" ]; then
            TEXT_DECREASE_MAX=$CHANGE
        fi
    fi

    if [ "$ACTUAL_DATA" -gt "$EXPECTED_DATA" ]; then
        CHANGE=$((ACTUAL_DATA - EXPECTED_DATA))
        echo "  data: $EXPECTED_DATA => $ACTUAL_DATA (INCREASE of $CHANGE bytes)"
        ((++DATA_INCREASE_COUNT))

        if [ -z "$DATA_INCREASE_MIN" ] || [ "$CHANGE" -lt "$DATA_INCREASE_MIN" ]; then
            DATA_INCREASE_MIN=$CHANGE
        fi
        if [ -z "$DATA_INCREASE_MAX" ] || [ "$CHANGE" -gt "$DATA_INCREASE_MAX" ]; then
            DATA_INCREASE_MAX=$CHANGE
        fi
    elif [ "$ACTUAL_DATA" -lt "$EXPECTED_DATA" ]; then
        CHANGE=$((EXPECTED_DATA - ACTUAL_DATA))
        echo "  data: $EXPECTED_DATA => $ACTUAL_DATA (DECREASE of $CHANGE bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_data[[:space:]]*=[[:space:]]*[0-9]+/expected_data = $ACTUAL_DATA/
            }" \
            "$MANIFEST"
        ((++DATA_DECREASE_COUNT))

        if [ -z "$DATA_DECREASE_MIN" ] || [ "$CHANGE" -lt "$DATA_DECREASE_MIN" ]; then
            DATA_DECREASE_MIN=$CHANGE
        fi
        if [ -z "$DATA_DECREASE_MAX" ] || [ "$CHANGE" -gt "$DATA_DECREASE_MAX" ]; then
            DATA_DECREASE_MAX=$CHANGE
        fi
    fi

    if [ "$ACTUAL_BSS" -gt "$EXPECTED_BSS" ]; then
        CHANGE=$((ACTUAL_BSS - EXPECTED_BSS))
        echo "  bss: $EXPECTED_BSS => $ACTUAL_BSS (INCREASE of $CHANGE bytes)"
        ((++BSS_INCREASE_COUNT))

        if [ -z "$BSS_INCREASE_MIN" ] || [ "$CHANGE" -lt "$BSS_INCREASE_MIN" ]; then
            BSS_INCREASE_MIN=$CHANGE
        fi
        if [ -z "$BSS_INCREASE_MAX" ] || [ "$CHANGE" -gt "$BSS_INCREASE_MAX" ]; then
            BSS_INCREASE_MAX=$CHANGE
        fi
    elif [ "$ACTUAL_BSS" -lt "$EXPECTED_BSS" ]; then
        CHANGE=$((EXPECTED_BSS - ACTUAL_BSS))
        echo "  bss: $EXPECTED_BSS => $ACTUAL_BSS (DECREASE of $CHANGE bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_bss[[:space:]]*=[[:space:]]*[0-9]+/expected_bss = $ACTUAL_BSS/
            }" \
            "$MANIFEST"
        ((++BSS_DECREASE_COUNT))

        if [ -z "$BSS_DECREASE_MIN" ] || [ "$CHANGE" -lt "$BSS_DECREASE_MIN" ]; then
            BSS_DECREASE_MIN=$CHANGE
        fi
        if [ -z "$BSS_DECREASE_MAX" ] || [ "$CHANGE" -gt "$BSS_DECREASE_MAX" ]; then
            BSS_DECREASE_MAX=$CHANGE
        fi
    fi
done < <(
    cargo metadata --no-deps --format-version 1 |
        jq -er '
            .packages[]
            | select(any(.targets[]; .kind[] == "bin"))
            | if .metadata.size_regression_test.skipped == true then
                empty
              elif
                .metadata.size_regression_test.expected_text != null and
                .metadata.size_regression_test.expected_data != null and
                .metadata.size_regression_test.expected_bss != null
              then
                [
                    (.targets[] | select(.kind[] == "bin") | .name),
                    .manifest_path,
                    .metadata.size_regression_test.expected_text,
                    .metadata.size_regression_test.expected_data,
                    .metadata.size_regression_test.expected_bss
                ] | @tsv
              else
                error(
                    "Binary package \(.name) has incomplete size_regression_test metadata"
                )
              end
        '
)

[ "$TEXT_INCREASE_COUNT" -eq 0 ] || echo "$TEXT_INCREASE_COUNT text increases (${TEXT_INCREASE_MIN}bytes - ${TEXT_INCREASE_MAX}bytes)"
[ "$TEXT_DECREASE_COUNT" -eq 0 ] || echo "$TEXT_DECREASE_COUNT text decreases (${TEXT_DECREASE_MIN}bytes - ${TEXT_DECREASE_MAX}bytes)"
[ "$DATA_INCREASE_COUNT" -eq 0 ] || echo "$DATA_INCREASE_COUNT data increases (${DATA_INCREASE_MIN}bytes - ${DATA_INCREASE_MAX}bytes)"
[ "$DATA_DECREASE_COUNT" -eq 0 ] || echo "$DATA_DECREASE_COUNT data decreases (${DATA_DECREASE_MIN}bytes - ${DATA_DECREASE_MAX}bytes)"
[ "$BSS_INCREASE_COUNT" -eq 0 ] || echo "$BSS_INCREASE_COUNT bss increases (${BSS_INCREASE_MIN}bytes - ${BSS_INCREASE_MAX}bytes)"
[ "$BSS_DECREASE_COUNT" -eq 0 ] || echo "$BSS_DECREASE_COUNT bss decreases (${BSS_DECREASE_MIN}bytes - ${BSS_DECREASE_MAX}bytes)"

if [ "$TEXT_INCREASE_COUNT" -eq 0 ] && [ "$DATA_INCREASE_COUNT" -eq 0 ] && [ "$BSS_INCREASE_COUNT" -eq 0 ]
then
    echo "SUCCESS"
    exit 0
else
    exit 1
fi