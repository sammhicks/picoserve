#!/usr/bin/env bash

set -eo pipefail

cd "$(dirname "$0")"

TEXT_INCREASE_COUNT=0
TEXT_DECREASE_COUNT=0
DATA_INCREASE_COUNT=0
DATA_DECREASE_COUNT=0
BSS_INCREASE_COUNT=0
BSS_DECREASE_COUNT=0

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
        echo "  text: $EXPECTED_TEXT => $ACTUAL_TEXT (INCREASE of $(($ACTUAL_TEXT - $EXPECTED_TEXT)) bytes)"
        ((++TEXT_INCREASE_COUNT))
    elif [ "$ACTUAL_TEXT" -lt "$EXPECTED_TEXT" ]; then
        echo "  text: $EXPECTED_TEXT => $ACTUAL_TEXT (DECREASE of $(($EXPECTED_TEXT - $ACTUAL_TEXT)) bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_text[[:space:]]*=[[:space:]]*[0-9]+/expected_text = $ACTUAL_TEXT/
            }" \
            "$MANIFEST"
        ((++TEXT_DECREASE_COUNT))
    fi

    if [ "$ACTUAL_DATA" -gt "$EXPECTED_DATA" ]; then
        echo "  data: $EXPECTED_DATA => $ACTUAL_DATA (INCREASE of $(($ACTUAL_DATA - $EXPECTED_DATA)) bytes)"
        ((++DATA_INCREASE_COUNT))
    elif [ "$ACTUAL_DATA" -lt "$EXPECTED_DATA" ]; then
        echo "  data: $EXPECTED_DATA => $ACTUAL_DATA (DECREASE of $(($EXPECTED_DATA - $ACTUAL_DATA)) bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_data[[:space:]]*=[[:space:]]*[0-9]+/expected_data = $ACTUAL_DATA/
            }" \
            "$MANIFEST"
        ((++DATA_DECREASE_COUNT))
    fi

    if [ "$ACTUAL_BSS" -gt "$EXPECTED_BSS" ]; then
        echo "  bss:  $EXPECTED_BSS => $ACTUAL_BSS (INCREASE of $(($ACTUAL_BSS - $EXPECTED_BSS)) bytes)"
        ((++BSS_INCREASE_COUNT))
    elif [ "$ACTUAL_BSS" -lt "$EXPECTED_BSS" ]; then
        echo "  bss:  $EXPECTED_BSS => $ACTUAL_BSS (DECREASE of $(($EXPECTED_BSS - $ACTUAL_BSS)) bytes)"
        sed -i -E \
            "/^\[package\.metadata\.size_regression_test\]$/,/^\[/ {
                s/^expected_bss[[:space:]]*=[[:space:]]*[0-9]+/expected_bss = $ACTUAL_BSS/
            }" \
            "$MANIFEST"
        ((++BSS_DECREASE_COUNT))
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

[ "$TEXT_INCREASE_COUNT" -eq 0 ] || echo "$TEXT_INCREASE_COUNT text increases"
[ "$TEXT_DECREASE_COUNT" -eq 0 ] || echo "$TEXT_DECREASE_COUNT text decreases"
[ "$DATA_INCREASE_COUNT" -eq 0 ] || echo "$DATA_INCREASE_COUNT data increases"
[ "$DATA_DECREASE_COUNT" -eq 0 ] || echo "$DATA_DECREASE_COUNT data decreases"
[ "$BSS_INCREASE_COUNT" -eq 0 ] || echo "$BSS_INCREASE_COUNT bss increases"
[ "$BSS_DECREASE_COUNT" -eq 0 ] || echo "$BSS_DECREASE_COUNT bss decreases"

if [ "$TEXT_INCREASE_COUNT" -eq 0 ] && [ "$DATA_INCREASE_COUNT" -eq 0 ] && [ "$BSS_INCREASE_COUNT" -eq 0 ]
then
    echo "SUCCESS"
    exit 0
else
    exit 1
fi