#!/bin/bash

set -xeuo pipefail

make
exec cargo run "$@"
