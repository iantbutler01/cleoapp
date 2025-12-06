CLIENT_BIN=target/debug/cleo
APP_NAME=Cleo.app
APP_DIR=target/debug/$(APP_NAME)
APP_ABSPATH=$(abspath $(APP_DIR))
APP_CONTENTS=$(APP_DIR)/Contents
APP_MACOS=$(APP_CONTENTS)/MacOS
INFO_PLIST=Info.plist
ENTITLEMENTS=client.entitlements
SIGN_IDENTITY?=Nick Gregory
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

.PHONY: build
build: bundle

.PHONY: bundle
bundle: $(INFO_PLIST) $(ENTITLEMENTS)
	cargo build
	rm -rf $(APP_DIR)
	mkdir -p $(APP_MACOS)
	install -m 755 $(CLIENT_BIN) $(APP_MACOS)/cleo
	install -m 644 $(INFO_PLIST) $(APP_CONTENTS)/Info.plist
	codesign --force --deep --sign "$(SIGN_IDENTITY)" --entitlements $(ENTITLEMENTS) $(APP_DIR)
	$(LSREGISTER) -f $(APP_ABSPATH)

.PHONY: run
run: build
	@if pgrep -x "Cleo" >/dev/null; then \
		echo "Killing existing app"; \
		killall "Cleo"; \
	fi
	open -n $(APP_ABSPATH)
