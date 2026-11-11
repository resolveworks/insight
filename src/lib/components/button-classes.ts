export type ButtonVariant = 'solid' | 'outline' | 'ghost';
export type ButtonSize = 'sm' | 'md' | 'icon';

const baseClasses =
	'inline-flex items-center justify-center gap-2 rounded-md border border-transparent font-medium transition-colors cursor-pointer whitespace-nowrap focus-visible:focus-ring disabled:opacity-50 disabled:cursor-not-allowed disabled:pointer-events-none';

const variantClasses: Record<ButtonVariant, string> = {
	solid: 'bg-neutral-900 text-white hover:bg-neutral-800',
	outline: 'border-neutral-300 bg-white text-neutral-800 hover:bg-neutral-100',
	// Translucent so the hover reads on both the neutral-100 page and white cards.
	ghost: 'text-neutral-600 hover:bg-neutral-900/5 hover:text-neutral-800',
};

const sizeClasses: Record<ButtonSize, string> = {
	sm: 'px-3 py-1.5 text-sm',
	md: 'px-4 py-2 text-sm',
	icon: 'p-1.5',
};

export function buttonClasses(
	variant: ButtonVariant,
	size: ButtonSize,
): string {
	return `${baseClasses} ${variantClasses[variant]} ${sizeClasses[size]}`;
}
