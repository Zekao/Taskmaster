#!/bin/bash

declare -a files=(
    "env_prg=env.c"
    "signal_prg=signal.c"
    "umask_prg=umask.c"
    "failure_prg=failure.c"
    "wait_prg=wait.c"
)

for file in "${files[@]}"; do
    IFS='=' read -r filename sourcefile <<< "$file"

    if [ -f "config/$filename" ]; then
        rm "config/$filename"
    fi

    cc "tests/$sourcefile" -o "config/$filename"
done
