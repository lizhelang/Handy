import InputiaMark from "./InputiaMark";

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
      <InputiaMark
        size={isHero ? 64 : 36}
        className="block shrink-0 text-accent-text"
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
