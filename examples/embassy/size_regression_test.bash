#!/usr/bin/env bash

set -eo pipefail

cd "$(dirname "$0")"

FAILURE_COUNT=0

while IFS=$'\t' read -r EXAMPLE EXPECTED
do
    echo "$EXAMPLE"

    cargo clean --release --target thumbv6m-none-eabi --package "$EXAMPLE"

    ACTUAL=$(RUSTFLAGS="-Zprint-type-sizes" cargo build --release --bin "$EXAMPLE" 2>&1 |
        grep 'type: `{async fn body of __web_task_task::__web_task_task_inner_function()}' |
        grep -oP '\d+(?= bytes, alignment:)') || ACTUAL=null

    if [ "$ACTUAL" = "null" ]
    then
        echo "$EXAMPLE - FAILED TO DETERMINE SIZE"
        ((FAILURE_COUNT++))
    elif [ "$ACTUAL" -gt "$EXPECTED" ]
    then
        echo "$EXAMPLE - SIZE INCREASE - $EXPECTED => $ACTUAL"
        ((FAILURE_COUNT++))
    elif [ "$ACTUAL" -lt "$EXPECTED" ]
    then
        echo "$EXAMPLE - SIZE DECREASE - $EXPECTED => $ACTUAL"

        # Update expected_size in this package's Cargo.toml.
        cargo metadata --no-deps --format-version 1 |
            jq -er --arg example "$EXAMPLE" '
                .packages[]
                | select(.targets[] | select(.kind[] == "bin") | .name == $example)
                | .manifest_path
            ' |
            xargs sed -i -E \
                "s/^([[:space:]]*expected_size[[:space:]]*=[[:space:]]*)[0-9]+/\1$ACTUAL/"
    fi
done < <(
    cargo metadata --no-deps --format-version 1 |
        jq -er '
            .packages[]
            | select(any(.targets[]; .kind[] == "bin"))
            | if .metadata.size_regression_test.skipped == true then
                empty
              elif .metadata.size_regression_test.expected_size != null then
                [
                    (.targets[] | select(.kind[] == "bin") | .name),
                    .metadata.size_regression_test.expected_size
                ] | @tsv
              else
                error("Binary package \(.name) has neither size_regression_test.expected_size nor size_regression_test.skipped = true")
              end
        '
)

if [ "$FAILURE_COUNT" -eq 0 ]
then
    echo "SUCCESS"
    exit 0
else
    echo "$FAILURE_COUNT TESTS FAILED"
    exit 1
fi