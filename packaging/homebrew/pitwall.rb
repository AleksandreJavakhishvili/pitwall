# Homebrew cask for Pitwall: a template, not published yet.
#
# To publish (once a release exists):
#   1. Create the tap repository github.com/AleksandreJavakhishvili/homebrew-tap
#      with a Casks/ folder.
#   2. Copy this file to Casks/pitwall.rb there and set `version` and `sha256`
#      from the release's SHA256SUMS.txt (the line for the .dmg).
#   3. Check it: brew audit --cask --new aleksandrejavakhishvili/tap/pitwall
#                brew install --cask aleksandrejavakhishvili/tap/pitwall
#   Users then install with:
#      brew install --cask aleksandrejavakhishvili/tap/pitwall
#
# Bump version + sha256 for every release (or automate it from release.yml).
cask "pitwall" do
  version "0.1.0"
  sha256 "REPLACE_WITH_SHA256_OF_THE_DMG"

  url "https://github.com/AleksandreJavakhishvili/pitwall/releases/download/v#{version}/Pitwall_#{version}_universal.dmg"
  name "Pitwall"
  desc "Hosts terminal coding agents and shows which one needs you"
  homepage "https://aleksandrejavakhishvili.github.io/pitwall/"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Pitwall.app"

  # Releases aren't notarized yet: without this, Gatekeeper blocks the first
  # launch. Remove once releases are Developer ID signed and notarized.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Pitwall.app"],
                   sudo: false
  end

  zap trash: [
    "~/.pitwall",
    "~/Library/Application Support/dev.pitwall.app",
    "~/Library/Caches/dev.pitwall.app",
    "~/Library/WebKit/dev.pitwall.app",
  ]
end
