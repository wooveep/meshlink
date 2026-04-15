SHELL := /bin/bash

.PHONY: proto fmt lint server client build-server build-client package-deb package-windows windows-runtime test smoke smoke-phase02 vm-lab vm-lab-phase03 vm-lab-phase03-deb vm-lab-phase08 tree

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

build-server:
	./scripts/build-server.sh

build-client:
	./scripts/build-client.sh

package-deb:
	./scripts/package-deb.sh

package-windows:
	./scripts/package-windows.sh

windows-runtime:
	pwsh -NoProfile -File ./scripts/build-wireguard-windows-runtime.ps1

test:
	./scripts/test-e2e.sh

smoke:
	./tests/e2e/phase01-smoke.sh

smoke-phase02:
	./tests/e2e/phase02-smoke.sh

vm-lab:
	./tests/nat-lab/run-phase01-02.sh

vm-lab-phase03:
	./tests/nat-lab/run-phase03.sh

vm-lab-phase03-deb:
	./tests/nat-lab/run-phase03-deb.sh

vm-lab-phase08:
	./tests/nat-lab/run-phase08-routes.sh

tree:
	find . -maxdepth 3 | sort
