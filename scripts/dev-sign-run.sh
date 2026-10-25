#!/bin/sh
# Cargo's runner on macOS. An ad-hoc signed debug build is a new app to the
# Keychain after every rebuild, so each one asks for the login password again.
# Zephium's own dev executables are re-signed with one identifier and the
# developer's certificate, which the Keychain keeps trusting across rebuilds
# after one "Always Allow". Without a certificate they run unchanged.
set -eu

bin=$1
shift

case $(basename -- "$bin") in
    zephium-desktop | macos-terra-agentic-probe | macos-agentic-liveness-probe) ;;
    *) exec "$bin" "$@" ;;
esac

identifier=app.quasar.dev
if ! /usr/bin/codesign -dv "$bin" 2>&1 | grep -qx "Identifier=$identifier"; then
    identity=${APPLE_SIGNING_IDENTITY:-}
    if [ -z "$identity" ]; then
        identity=$(security find-identity -v -p codesigning 2>/dev/null |
            sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' | head -n 1)
    fi
    if [ -n "$identity" ]; then
        /usr/bin/codesign --force --sign "$identity" --identifier "$identifier" \
            --timestamp=none "$bin" 2>/dev/null ||
            echo "dev-sign-run: could not sign $bin; the Keychain may ask again" >&2
    fi
fi

exec "$bin" "$@"
