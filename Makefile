CARGO ?= cargo
PACKAGE = byote

.DEFAULT_GOAL = all
.PHONY: clean clippy fmt all run

clean:
	$(CARGO) clean --package $(PACKAGE)

fmt:
	$(CARGO) fmt

clippy: clean
	$(CARGO) clippy -- -D warnings

all: fmt clippy
