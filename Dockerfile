ARG RUST_VERSION='1.79'
ARG RUST_TARGET='x86_64-unknown-linux-musl'
ARG BINARY_NAME='karaokify'

ARG APP_FEATURES=''

ARG RUN_USERNAME='app'
ARG RUN_USER_ID='1000'
ARG RUN_GROUP_ID='1000'

ARG FFMPEG_DOWNLOAD_URL='https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz'


##########
# Step 0 #
##########
##
## Setup base image with cargo-chef
##
FROM rust:${RUST_VERSION} AS chef
# `curl` and `bash` are needed for cargo-binstall
# `musl-tools` and `musl-dev` are needed to build app with musl target
RUN apt-get update && apt-get install -y \
  curl \
  bash \
  musl-tools \
  musl-dev \
  jq \
  && rm -rf /var/lib/apt/lists/*
# Install cargo-binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf 'https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh' | bash
# Install cargo-chef
RUN cargo binstall cargo-chef --locked --no-confirm
# Add proper target and compile flags
ARG RUST_TARGET
RUN rustup target add "${RUST_TARGET}"
ENV RUSTFLAGS='-C target-feature=+crt-static'
WORKDIR /app


##########
# Step 1 #
##########
##
## Generate a build plan for rust dependencies
##
FROM chef AS planner
COPY . .
# Generate "lockfile" aka dependency dump
RUN cargo chef prepare \
  --recipe-path recipe.json


##########
# Step 2 #
##########
##
## Build app with the cached dependencies
##
FROM chef AS builder
# Install upx - https://upx.github.io/
RUN cd "$(mktemp --directory)" && \
  curl -sL "$(\
  curl -sL https://api.github.com/repos/upx/upx/releases \
  | jq -r '.[0].assets | .[] | select(.name | test("amd64_linux")) | .browser_download_url' \
  | head -n1\
  )" | tar xvJ  && \
  cd * && \
  mv upx /usr/bin && \
  cd .. && \
  rm -rf "$(pwd)" && \
  echo "Installed upx"
COPY --from=planner /app/recipe.json .
# Build dependencies
ARG RUST_TARGET
ARG APP_FEATURES
ARG BINARY_NAME
RUN cargo chef cook \
  --release \
  --target "${RUST_TARGET}" \
  --features "${APP_FEATURES}" \
  --package "${BINARY_NAME}" \
  --recipe-path recipe.json
ARG RUST_TARGET
RUN rustup target add "${RUST_TARGET}"
# Copy rest of files and compile
# only the remaining app code
COPY . .
ARG RUST_TARGET
ARG APP_FEATURES
ARG BINARY_NAME
RUN cargo build \
  --release \
  --target "${RUST_TARGET}" \
  --features "${APP_FEATURES}" \
  --package "${BINARY_NAME}"
RUN upx --best --lzma "/app/target/${RUST_TARGET}/release/${BINARY_NAME}"


##########
# Step 3 #
##########
##
## Run the app in a configured environment
##
FROM ubuntu:rolling as runner
RUN apt-get update && apt-get install -y \
  sudo \
  curl \
  fontconfig \
  bzip2 \
  python3-full \
  python3-pip \
  xattr \
  xz-utils \
  && rm -rf /var/lib/apt/lists/* \
  && echo "Done installing packages"
# Install latest ffmpeg
ARG FFMPEG_DOWNLOAD_URL
RUN cd "$(mktemp --directory)" && \
  curl -svL "${FFMPEG_DOWNLOAD_URL}" | tar xvJ \
  && cd ffmpeg-*-amd64-static \
  && mv ffmpeg ffprobe qt-faststart /usr/local/bin/ \
  && cd .. \
  && rm -rf "$(pwd)"
# Install demucs
RUN python3 -m pip install -U soundfile demucs --break-system-packages
# Delete default ubuntu user
RUN userdel --remove ubuntu; groupdel ubuntu; echo "Deleted default ubuntu user"
# Create run user
ARG RUN_USERNAME
ARG RUN_USER_ID
ARG RUN_GROUP_ID
RUN groupadd --gid "${RUN_GROUP_ID}" "${RUN_USERNAME}"
RUN useradd --create-home --uid "${RUN_USER_ID}" --gid "${RUN_GROUP_ID}" "${RUN_USERNAME}"
# Test demucs and cache model
USER ${RUN_USERNAME}
COPY --chmod=777 --chown=${RUN_USERNAME}:${RUN_USERNAME} test.mp3 /tmp/test.mp3
RUN demucs -v -n 'htdemucs' /tmp/test.mp3 --out /tmp/ && rm -rf /tmp/*
USER root
# Install app
ARG RUST_TARGET
ARG BINARY_NAME
COPY --from=builder "/app/target/${RUST_TARGET}/release/${BINARY_NAME}" /usr/local/bin/
RUN chmod a=rx "/usr/local/bin/${BINARY_NAME}"
# Run app
RUN echo "#!/bin/bash\n\n/usr/local/bin/${BINARY_NAME} \"\$@\"" > /entrypoint.sh && chmod +x /entrypoint.sh
USER ${RUN_USERNAME}
LABEL maintainer="Josip Igrec <me@allypost.net>"
LABEL org.opencontainers.image.title="karaokify"
LABEL org.opencontainers.image.description="Telegram bot to download music and split into tracks"
LABEL org.opencontainers.image.source="https://github.com/Allypost/karaokify-rs"
LABEL org.opencontainers.image.licenses="MPL-2.0"
LABEL org.opencontainers.image.authors="Josip Igrec <me@allypost.net>"
EXPOSE 8000
ENTRYPOINT [ "/entrypoint.sh" ]
