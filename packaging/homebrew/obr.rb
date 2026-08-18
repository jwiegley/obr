# frozen_string_literal: true

# Homebrew formula for obr - Org-file issue tracker
# Repository: https://github.com/jwiegley/obr
#
# To install:
#   brew tap jwiegley/tap
#   brew install obr
#
# Or directly:
#   brew install jwiegley/tap/obr
#
# The sha256 values below are zero placeholders until obr publishes its first
# release; `.github/workflows/update-package-manifests.yml` rewrites them from
# the published `.sha256` sidecars. A zero digest fails the install loudly
# rather than letting a wrong artifact through, which is the point.

class Obr < Formula
  desc "Issue tracker with an Org-file surface (SQLite cache + Org export)"
  homepage "https://github.com/jwiegley/obr"
  # `0.5.7+1` is upstream beads_rust release this fork tracks, plus fork
  # generation since it.
  version "0.5.7+1"
  # The additional rider makes this license non-SPDX.
  license :cannot_represent

  # The published archives spell the `+` as `.`. GitHub's release-asset upload
  # API takes the asset name as a URL query parameter, where a literal `+`
  # decodes to a space and is stored back as `.`, so `release.yml` flattens it
  # before uploading and the file on the release is `obr-0.5.7.1-*`.
  # Only the file name is flattened; the tag keeps the true version.
  on_macos do
    on_arm do
      url "https://github.com/jwiegley/obr/releases/download/v#{version}/obr-#{version.to_s.tr("+", ".")}-darwin_arm64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"  # darwin_arm64
    end
    on_intel do
      url "https://github.com/jwiegley/obr/releases/download/v#{version}/obr-#{version.to_s.tr("+", ".")}-darwin_amd64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"  # darwin_amd64
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/jwiegley/obr/releases/download/v#{version}/obr-#{version.to_s.tr("+", ".")}-linux_arm64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"  # linux_arm64
    end
    on_intel do
      url "https://github.com/jwiegley/obr/releases/download/v#{version}/obr-#{version.to_s.tr("+", ".")}-linux_amd64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"  # linux_amd64
    end
  end

  def install
    bin.install "obr"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/obr --version")

    # Test basic functionality
    system bin/"obr", "init"
    assert_predicate testpath/".obr", :directory?
    assert_predicate testpath/".obr/obr.db", :file?
    # The tracked surface lives outside the cache directory, in the project.
    assert_predicate testpath/"PLAN.org", :file?
  end
end
