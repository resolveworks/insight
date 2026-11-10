export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md';

const baseClasses =
	'inline-flex items-center justify-center gap-2 rounded-md border border-transparent font-medium no-underline hover:no-underline transition-colors cursor-pointer whitespace-nowrap focus-visible:focus-ring disabled:opacity-50 disabled:cursor-not-allowed disabled:pointer-events-none';

const variantClasses: Record<ButtonVariant, string> = {
	primary: 'bg-blue-600 text-white hover:bg-blue-700',
	secondary:
		'border-neutral-300 bg-white text-neutral-800 hover:bg-neutral-100',
	ghost: 'text-neutral-700 hover:bg-neutral-100',
	danger: 'bg-red-600 text-white hover:bg-red-700',
};

const sizeClasses: Record<ButtonSize, string> = {
	sm: 'px-3 py-1.5 text-sm',
	md: 'px-4 py-2 text-sm',
};

export function buttonClasses(
	variant: ButtonVariant,
	size: ButtonSize,
): string {
	return `${baseClasses} ${variantClasses[variant]} ${sizeClasses[size]}`;
}
