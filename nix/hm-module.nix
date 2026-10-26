# Home-manager module for Inputia speech-to-text
#
# Provides a systemd user service for autostart.
# Usage: imports = [ inputia.homeManagerModules.default ];
#        services.handy.enable = true;
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.handy;
in
{
  options.services.handy = {
    enable = lib.mkEnableOption "Inputia speech-to-text user service";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "inputia.packages.\${system}.handy";
      description = "The Inputia package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.handy = {
      Unit = {
        Description = "Inputia speech-to-text";
        After = [ "graphical-session.target" ];
        PartOf = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/inputia";
        Restart = "on-failure";
        RestartSec = 5;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
