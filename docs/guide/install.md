# Install

The quickest way in is the installer on the [Download](../download.md) page. To
build from source instead, there is one setup step per tool, and the first build
takes a while.

## What to install

1. [Rust](https://rustup.rs/), using the stable toolchain. On Windows, `rustup`
   also offers to install the Microsoft C++ build tools, which you need.
2. [Git](https://git-scm.com/).
3. LLVM, for ASIO support:

    ```bash
    winget install LLVM.LLVM
    ```

    The build looks for it in `C:\Program Files\LLVM\bin`. If you installed it
    somewhere else, set `LIBCLANG_PATH` to that folder.

## Build and run

From PowerShell, not Git Bash, because Git Bash puts its own `link.exe` ahead of
the Microsoft one.

```bash
git clone https://github.com/joeyf116/amp-gpui.git
```

```bash
cd amp-gpui
```

```bash
cargo run -p amp-app --release
```

The first build downloads Steinberg's ASIO SDK, which the ASIO support compiles
against. Later builds are much quicker.

!!! tip "Skipping ASIO"

    If you don't want LLVM or the SDK, turn ASIO off. You'll still have the two
    Windows audio routes.

    ```bash
    cargo run -p amp-app --release --no-default-features
    ```

## Where your data goes

The app keeps its settings and your tone library here:

```text
%APPDATA%\amp-gpui\data\settings.json
%APPDATA%\amp-gpui\data\library\
```

You can move the library anywhere with the **Folder** button. See
[Share tones](sharing.md).

Next, [connect your interface](setup.md).

<small>![ASIO compatible](../assets/asio-compatible.png){ .asio-logo width="80" }<br>ASIO is a registered trademark of Steinberg Media Technologies GmbH.</small>
