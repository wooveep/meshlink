SHELL := /bin/bash

.PHONY: proto fmt lint server client test smoke smoke-phase02 vm-lab tree

proto:
	./scripts/gen-proto.sh

fmt:
	./scripts/fmt.sh

lint:
	./scripts/lint.sh

server:
	cd server && go build ./...

client:
	cd client && cargo build

test:
	./scripts/test-e2e.sh

smoke:
	./tests/e2e/phase01-smoke.sh

smoke-phase02:
	./tests/e2e/phase02-smoke.sh

vm-lab:
	./tests/nat-lab/run-phase01-02.sh

tree:
	find . -maxdepth 3 | sort
