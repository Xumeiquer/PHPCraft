# PHPCraft developer tasks.
#
#   make            build the workspace
#   make check      formatting, clippy, unit tests and differential tests
#   make run FILE=path/to/script.php
#   make help       list every target

CARGO ?= cargo
PHP ?= php
PHPCRAFT ?= target/debug/phpcraft
FILE ?=
LIMIT ?= 150

.PHONY: all build release test diff phpt fmt fmt-check clippy check run repl clean help

all: build

## Build the debug binary.
build:
	$(CARGO) build

## Build an optimized binary.
release:
	$(CARGO) build --release

## Run the Rust unit and integration tests.
test:
	$(CARGO) test --workspace

## Run tests/php against the reference PHP interpreter.
diff: build
	PHP=$(PHP) PHPCRAFT=$(PHPCRAFT) scripts/diff-test.sh

## Extract php-src --FILE-- sections and run them differentially.
## Requires vendor/php-src (see README). LIMIT=0 extracts everything.
phpt: build
	scripts/phpt-subset.sh --limit $(LIMIT)
	PHP=$(PHP) PHPCRAFT=$(PHPCRAFT) scripts/diff-test.sh tests/phpt

## Format the workspace.
fmt:
	$(CARGO) fmt --all

## Fail if formatting is not clean.
fmt-check:
	$(CARGO) fmt --all -- --check

## Lint with warnings treated as errors.
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

## The full gate: format, lint, unit tests and differential tests.
check: fmt-check clippy test diff

## Run one script: make run FILE=tests/php/01_basics.php
run: build
	@test -n "$(FILE)" || { echo "usage: make run FILE=path/to/script.php" >&2; exit 2; }
	$(PHPCRAFT) $(FILE)

## Evaluate a snippet: make repl CODE='echo 1 + 2, PHP_EOL;'
repl: build
	@test -n "$(CODE)" || { echo "usage: make repl CODE='echo 1 + 2;'" >&2; exit 2; }
	$(PHPCRAFT) -r '$(CODE)'

## Remove build products and generated php-src extracts.
clean:
	$(CARGO) clean
	rm -rf tests/phpt

## Show this help.
help:
	@awk 'BEGIN { print "PHPCraft targets:" } \
		/^## / { if (help == "") { sub(/^## /, ""); help = $$0 } next } \
		/^[a-zA-Z0-9_-]+:/ { if (help != "") { split($$1, t, ":"); printf "  %-12s %s\n", t[1], help; help = "" } } \
		END { print ""; print "Variables: FILE=<script> CODE=<snippet> LIMIT=<n> PHP=<binary>" }' $(MAKEFILE_LIST)
