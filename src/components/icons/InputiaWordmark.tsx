import inputiaLogoUrl from "../../../macos/InputiaInputMethod/Resources/InputiaLogo.svg";

const PRODUCT_NAME = "Inputia";

interface InputiaWordmarkProps {
  size?: "sidebar" | "hero";
  className?: string;
}

const InputiaWordmark = ({
  size = "sidebar",
  className = "",
}: InputiaWordmarkProps) => {
  const isHero = size === "hero";

  return (
    <div
      className={`flex shrink-0 items-center justify-center ${isHero ? "gap-4" : "gap-2"} ${className}`}
      aria-label={PRODUCT_NAME}
    >
      <span
        data-inputia-mark=""
        aria-hidden="true"
        className={`block shrink-0 bg-accent-text ${isHero ? "h-16 w-16" : "h-9 w-9"}`}
        style={{
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
      <span
        className={`whitespace-nowrap font-semibold tracking-normal text-text ${
          isHero ? "text-5xl" : "text-xl"
        }`}
      >
        {PRODUCT_NAME}
      </span>
    </div>
  );
};

export default InputiaWordmark;
