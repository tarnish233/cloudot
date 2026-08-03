cask "cloudot" do
  version "0.4.1"
  sha256 "5bffbe664630b8b0294953ca2889e7068ca1202500f2542528f8df53d36e3b38"

  url "https://github.com/tarnish233/cloudot/releases/download/v#{version}/Cloudot-#{version}.dmg"
  name "cloudot"
  desc "Sync dotfiles across Macs through Git"
  homepage "https://github.com/tarnish233/cloudot"

  depends_on macos: :sequoia

  app "Cloudot.app"
  # .app 里自带一份 CLI（GUI 优先用它，用户不必先装命令行工具）。
  # 顺手链到 PATH，这样 `brew install --cask cloudot` 一条命令就同时装齐 GUI 和 CLI。
  binary "#{appdir}/Cloudot.app/Contents/Resources/cloudot"

  zap trash: [
    "~/Library/Preferences/com.tarnish233.cloudot.plist",
    "~/Library/Saved Application State/com.tarnish233.cloudot.savedState",
  ]

  caveats <<~EOS
    cloudot 没有 Apple 开发者签名，装完先跑这一行：

      xattr -dr com.apple.quarantine #{appdir}/Cloudot.app

    不跑的话 GUI 打不开，而且 `cloudot` 命令会被系统直接杀掉（退出码 137）——
    因为链到 PATH 上的那个二进制就在 .app 里面，同样带着隔离属性。

    它是纯菜单栏应用：启动后不占 Dock，图标在屏幕右上角。

    注意 zap 不会删 ~/.cloudot —— 那里是你的配置仓库和备份，
    要一起清掉的话自己 rm -rf ~/.cloudot。
  EOS
end
