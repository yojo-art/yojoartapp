set -eu
uid=$(stat -c "%u" .)
gid=$(stat -c "%g" .)
groupadd --gid $gid app
useradd --uid $uid --gid $gid app
mkdir /home/app
chown app:app /home/app

cat << EOF | su app -c bash
set -eu

export CARGO_HOME="/cache/cargo_cache"
mkdir -p \${CARGO_HOME}

export CC="x86_64-linux-android28-clang"
export RUSTFLAGS="-C linker=\${CC}"
export HOME="/cache/home_cache"
mkdir -p \${HOME}
x build -r --platform android --arch x64 --format apk
mv /app/target/x/release/android/yojo_art_app.apk /app/target/yojo_art_app-x86_64.apk

export CC="aarch64-linux-android28-clang"
export RUSTFLAGS="-C linker=\${CC}"
x build -r --platform android --arch arm64 --format apk
mv /app/target/x/release/android/yojo_art_app.apk /app/target/yojo_art_app-aarch64.apk

EOF
