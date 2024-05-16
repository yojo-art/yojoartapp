FROM rust:bookworm
RUN rustup target add aarch64-linux-android x86_64-linux-android
RUN apt-get update && apt-get install -y curl zip clang openjdk-17-jdk gradle
RUN mkdir /ndk
WORKDIR /ndk
RUN curl -sSL https://dl.google.com/android/repository/android-ndk-r26d-linux.zip > android-ndk-r26d-linux.zip && unzip android-ndk-r26d-linux.zip && rm android-ndk-r26d-linux.zip
ENV PATH $PATH:/ndk/android-ndk-r26d:/ndk/android-ndk-r26d/toolchains/llvm/prebuilt/linux-x86_64/bin
ENV ANDROID_NDK_HOME="/ndk/android-ndk-r26d"
RUN cargo install xbuild
#COPY Cargo.toml ./Cargo.toml
#COPY src ./src
#RUN --mount=type=cache,target=/var/cache/cargo --mount=type=cache,target=/app/target cargo build --target aarch64-linux-android --release
WORKDIR /app
CMD ["bash","build.sh"]
