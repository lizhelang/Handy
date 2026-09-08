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
      className={`flex items-center justify-center gap-3 ${className}`}
      aria-label={PRODUCT_NAME}
    >
      <div
        className={`rounded-2xl border border-logo-primary/40 bg-logo-primary/15 shadow-sm shadow-logo-primary/20 ${
          isHero ? "p-3.5" : "p-2"
        }`}
      >
        <img
          src={inputiaLogoUrl}
          alt=""
          aria-hidden="true"
          className={isHero ? "h-10 w-10" : "h-7 w-7"}
        />
      </div>
      <span
        className={`font-semibold tracking-normal text-text ${
          isHero ? "text-5xl" : "text-2xl"
        }`}
      >
        {PRODUCT_NAME}
      </span>
    </div>
  );
};

export default InputiaWordmark;
