.DEFAULT_GOAL := help
SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.ONESHELL:
PYTHON ?= python3
export DEBONAIR_VERSION ?= 0.0.0
DESKTOP_TARGETS ?= AppImage tar.gz
BOARD ?= pico_w
PROFILE ?= development
PROTOCOL_BASE ?= $(shell $(PYTHON) tools/version.py --protocol-base 2>/dev/null)
# Set when the base release has the serial schema, which buf breaking then compares against.
protocol_base_schema = $(if $(PROTOCOL_BASE),$(shell git cat-file -e '$(PROTOCOL_BASE):proto/debonair.proto' 2>/dev/null && echo yes))
# The base release's schemas as a buf input, from the repository's Git directory, which a worktree
# shares.
PROTOCOL_AGAINST ?= $(if $(protocol_base_schema),$(shell git rev-parse --path-format=absolute --git-common-dir)#format=git$(comma)ref=$(PROTOCOL_BASE)$(comma)subdir=proto)
# Set when PROTOCOL_BASE or PROTOCOL_AGAINST is given rather than derived from the version.
explicit_protocol_base = $(filter command environment,$(firstword $(origin PROTOCOL_BASE)) $(firstword $(origin PROTOCOL_AGAINST)))
# Fails when the version is invalid, a shallow clone finds no base release, or PROTOCOL_BASE names
# no commit, so none of them can skip buf breaking.
define check_protocol_base
$(if $(explicit_protocol_base),,$(PYTHON) tools/version.py --protocol-base > /dev/null)
if [ -n '$(PROTOCOL_BASE)' ] && ! git rev-parse -q --verify '$(PROTOCOL_BASE)^{commit}' > /dev/null; then
  echo 'PROTOCOL_BASE $(PROTOCOL_BASE) is not a commit' >&2
  exit 1
fi
endef
BOARDS := pico_w pico2_w waveshare_rp2350b_plus_w xiao_esp32s3
ESP_BOARDS := xiao_esp32s3
DOCKER ?= docker
FIRMWARE_PLATFORM = $(if $(filter $(ESP_BOARDS),$(BOARD)),esp32s3,pico)

.PHONY: help all desktop local-desktop firmware firmware-all package-desktop local-package-desktop \
	image-app image-pico image-esp32s3 check check-rust check-desktop check-protocol check-firmware \
	check-firmware-all docker-check-rust docker-check-desktop docker-check-protocol version clean
help:
	@echo 'Debonair: all desktop firmware firmware-all package-desktop check check-firmware check-firmware-all version clean'
	@echo 'Without Docker: local-desktop local-package-desktop check-rust check-desktop check-protocol'
	@echo 'Firmware options: BOARD=$(BOARD) PROFILE=$(PROFILE)'
	@echo 'Boards: $(BOARDS)'
all: desktop firmware-all

# Toolchain images from docker/. Each build copies the checkout into one of them (without the
# paths in .dockerignore), runs there, and exports only its outputs back into the checkout.
image-app:
	$(DOCKER) build -t debonair-app -f docker/app.Dockerfile docker
image-pico:
	$(DOCKER) build -t debonair-pico -f docker/pico.Dockerfile docker
image-esp32s3:
	$(DOCKER) build -t debonair-esp32s3 -f docker/esp32s3.Dockerfile docker
comma := ,
# $(call build,IMAGE,COMMAND,OUTPUTS[,INPUT]) runs COMMAND with bash in the image and exports the
# paths matching OUTPUTS, relative to the checkout; each replaces the checkout's copy. INPUT is a
# file or directory the build reads from /inputs. Each target has its own working directories
# under .cache/docker/, so parallel builds stay apart.
define build
rm -rf .cache/docker/$@
mkdir -p .cache/docker/$@/inputs
$(if $(4),cp -R '$(4)' .cache/docker/$@/inputs/)
$(DOCKER) build --progress=plain -f docker/build.Dockerfile --build-arg IMAGE=debonair-$(1) \
  --build-arg DEBONAIR_VERSION --build-arg SOURCE_DATE_EPOCH \
  --build-arg COMMAND='$(2)' --build-arg OUTPUTS='$(3)' --build-context inputs=.cache/docker/$@/inputs \
  --output type=local$(comma)dest=.cache/docker/$@/out .
for pattern in $(foreach p,$(3),'$(p)'); do
  for exported in .cache/docker/$@/out/$$pattern; do
    [ -e "$$exported" ] || continue
    path=$${exported#.cache/docker/$@/out/}
    rm -rf "$$path"
    mkdir -p "$$(dirname "$$path")"
    mv "$$exported" "$$path"
  done
done
endef

desktop: image-app
	$(call build,app,make local-desktop,desktop/out/main desktop/out/preload desktop/out/renderer)
package-desktop: image-app
	$(call build,app,make local-package-desktop,desktop/dist/*.AppImage desktop/dist/*.tar.gz)

desktop/node_modules/.package-lock.json: desktop/package.json desktop/package-lock.json $(wildcard desktop/packages/*/package.json)
	npm ci --include=dev --prefix desktop
local-desktop: desktop/node_modules/.package-lock.json
	npm --prefix desktop run build
local-package-desktop: local-desktop
	version=$$($(PYTHON) tools/version.py)
	printf '%s\n' "$$version" > desktop/out/VERSION
	cd desktop
	npm exec -- electron-builder --linux $(DESKTOP_TARGETS) --publish never --config.extraMetadata.version="$$version"

firmware: image-$(FIRMWARE_PLATFORM)
	$(call build,$(FIRMWARE_PLATFORM),python3 rust/tools/build_firmware.py rust/boards/$(BOARD).json --profile $(PROFILE),build/firmware/*/*)
firmware-all:
	@set -e; for board in $(BOARDS); do $(MAKE) firmware BOARD=$$board PROFILE=$(PROFILE); done

check: docker-check-rust docker-check-desktop docker-check-protocol
# Clippy on the firmware platform crate for BOARD and PROFILE, in the board's image.
check-firmware: image-$(FIRMWARE_PLATFORM)
	$(call build,$(FIRMWARE_PLATFORM),python3 rust/tools/build_firmware.py rust/boards/$(BOARD).json --profile $(PROFILE) --clippy)
check-firmware-all:
	@set -e; for board in $(BOARDS); do for profile in development production; do $(MAKE) check-firmware BOARD=$$board PROFILE=$$profile; done; done
docker-check-rust: image-app
	$(call build,app,make check-rust)
docker-check-desktop: image-app
	$(call build,app,make check-desktop)
# The image has no Git history, so the base release's schemas go in as an input.
docker-check-protocol: image-app
	$(check_protocol_base)
	rm -rf .cache/protocol-base
	mkdir -p .cache/protocol-base
	$(if $(protocol_base_schema),git archive '$(PROTOCOL_BASE)' proto | tar -x -C .cache/protocol-base)
	$(call build,app,make check-protocol PROTOCOL_AGAINST=$(if $(protocol_base_schema),/inputs/proto),,$(if $(protocol_base_schema),.cache/protocol-base/proto))
check-rust:
	$(PYTHON) rust/tools/firmware_dependencies.py --btstack
	cd rust
	cargo test --locked --workspace --all-features
	cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
check-desktop: desktop/node_modules/.package-lock.json
	npm --prefix desktop run check
# Lint and breaking checks cover every schema in proto/. A schema the base release lacks is new to
# buf breaking and has nothing to break.
check-protocol: desktop/node_modules/.package-lock.json
	$(check_protocol_base)
	npm --prefix desktop/packages/protocol run lint
	if [ -n '$(PROTOCOL_AGAINST)' ]; then
	  npm --prefix desktop/packages/protocol run breaking -- '$(PROTOCOL_AGAINST)'
	fi
version:
	@$(PYTHON) tools/version.py
clean:
	rm -rf rust/target rust/platforms/pico/target rust/platforms/esp32s3/target desktop/out desktop/dist build/firmware target/firmware target/tmp
