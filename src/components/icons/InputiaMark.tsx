import inputiaLogoUrl from "../../../macos/InputiaInputMethod/Resources/InputiaLogo.svg";

interface InputiaMarkProps {
  size?: number;
  className?: string;
}

const InputiaMark = ({ size = 24, className = "" }: InputiaMarkProps) => (
  <span
    className={className}
    data-inputia-mark=""
    aria-hidden="true"
    style={{
      display: "inline-block",
      width: size,
      height: size,
      backgroundColor: "currentColor",
      maskImage: `url(${JSON.stringify(inputiaLogoUrl)})`,
      WebkitMaskImage: `url(${JSON.stringify(inputiaLogoUrl)})`,
      maskSize: "contain",
      WebkitMaskSize: "contain",
      maskPosition: "center",
      WebkitMaskPosition: "center",
      maskRepeat: "no-repeat",
      WebkitMaskRepeat: "no-repeat",
    }}
  />
);

export default InputiaMark;
