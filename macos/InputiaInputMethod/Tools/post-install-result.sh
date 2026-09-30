#!/bin/sh
# 仅聚合已执行的步骤；本文件不操作应用、输入源或用户数据。
inputia_post_install_result() {
  if [ "$1" != "1" ]; then
    echo "postInstallRegressionPassed=false"
    echo "postInstallRegressionStatus=BLOCKED reason=ui-smoke-not-run"
    return 8
  fi
  if [ "$2" != "1" ] || [ "$3" != "0" ]; then
    echo "postInstallRegressionPassed=false"
    echo "postInstallRegressionStatus=BLOCKED reason=ui-smoke-incomplete-or-preflight-bypassed"
    return 8
  fi
  echo "postInstallRegressionPassed=true"
  echo "postInstallRegressionStatus=PASS evidenceLevel=native_api scope=post-install-smoke"
  echo "releaseAcceptancePassed=false reason=full-release-matrix-required"
}
