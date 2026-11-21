{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  makeWrapper,
  fcitx5,
  alsa-lib,
  wayland,
  libxkbcommon,
  libGL,
  withSettings ? false,
}:

let
  cargoTarget = "target/${stdenv.hostPlatform.rust.cargoShortTarget}/release";
in
rustPlatform.buildRustPackage {
  pname = if withSettings then "flype-all" else "flype";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.cleanSourceWith {
    src = ../.;
    filter =
      path: _type:
      let
        base = baseNameOf path;
      in
      !(builtins.elem base [
        "target"
        "dist"
        ".git"
        ".prux"
        "WowType"
        "__pycache__"
      ]);
  };

  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags =
    if withSettings then
      [
        "-p"
        "flype-fcitx5"
        "-p"
        "flype-config"
      ]
    else
      [
        "-p"
        "flype-fcitx5"
      ];

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    fcitx5
    alsa-lib
  ]
  ++ lib.optionals withSettings [
    wayland
    libxkbcommon
    libGL
  ];

  # build.rs 找 fcitx5：Nix 的包没有 .pc，给出库目录后它自己从库路径反推头文件目录
  FLYPE_FCITX_LIB_DIRS = "${fcitx5}/lib";

  # 编进插件的默认路径：设置程序、系统数据目录（都是这个包自己的位置）
  preBuild = ''
    export FLYPE_PREFIX=$out
    export FLYPE_CONFIG_TOOL=$out/bin/flype-config
    export FLYPE_SYSTEM_DATA_DIR=$out/share/flype
  '';

  doCheck = false;

  installPhase = ''
    runHook preInstall

    install -Dm755 ${cargoTarget}/libflype.so $out/lib/fcitx5/libflype.so
    install -Dm644 ${cargoTarget}/addon/flype-addon.conf $out/share/fcitx5/addon/flype.conf
    install -Dm644 ${cargoTarget}/addon/flype.conf $out/share/fcitx5/inputmethod/flype.conf

    # 雾凇词库：scripts/make-data.sh 生成在 data/flype（不进仓库时就是 demo 兜底）
    for f in lexicon.tsv english.txt VERSION LICENSE; do
      [ -f data/flype/$f ] && install -Dm644 data/flype/$f $out/share/flype/$f
    done

    # 万象：备选词库，设置里切过去（Lexicon=wanxiang）才用
    [ -f data/flype/wanxiang/lexicon.tsv ] && install -Dm644 data/flype/wanxiang/lexicon.tsv \
      $out/share/flype/dicts/wanxiang/lexicon.tsv

    # 测试词库
    install -Dm644 core/data/demo_lexicon.tsv $out/share/flype/demo_lexicon.tsv

    cp -r icons/hicolor $out/share/icons
    cp -r themes/fcitx5 $out/share/fcitx5/themes

    ${lib.optionalString withSettings ''
      install -Dm755 ${cargoTarget}/flype-config $out/bin/flype-config
      # winit/glutin 靠 dlopen 找 wayland、xkbcommon、EGL，autoPatchelf 只管 DT_NEEDED，得自己给
      wrapProgram $out/bin/flype-config \
        --prefix LD_LIBRARY_PATH : ${
          lib.makeLibraryPath [
            wayland
            libxkbcommon
            libGL
          ]
        }
      install -Dm644 settings/flype-config.desktop $out/share/applications/flype-config.desktop
    ''}

    runHook postInstall
  '';

  meta = {
    description = "Wayland 下的 fcitx5 拼音输入法（核心用 Rust，外壳用 C++）";
    homepage = "https://github.com/heng30/flype";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = if withSettings then "flype-config" else "libflype.so";
  };
}
