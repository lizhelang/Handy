interface InputiaMarkProps {
  size?: number;
  className?: string;
}

const InputiaMark = ({ size = 24, className = "" }: InputiaMarkProps) => (
  <svg
    width={size}
    height={size}
    viewBox="96 96 320 320"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    className={className}
    data-inputia-mark=""
    aria-hidden="true"
    focusable="false"
  >
    <circle cx="256" cy="256" r="142" stroke="currentColor" strokeWidth="34" />
    <path
      fill="currentColor"
      fillRule="evenodd"
      d="M298 256a42 42 0 1 1-84 0 42 42 0 1 1 84 0Zm-24 0a18 18 0 1 0-36 0 18 18 0 1 0 36 0Z"
    />
  </svg>
);

export default InputiaMark;
