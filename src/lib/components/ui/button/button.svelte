<script lang="ts" module>
	import { tv, type VariantProps } from 'tailwind-variants';

	export const buttonVariants = tv({
		base: 'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40 cursor-pointer select-none',
		variants: {
			variant: {
				default: 'bg-primary text-primary-foreground hover:bg-primary/90',
				secondary: 'bg-secondary text-secondary-foreground hover:bg-secondary/80',
				ghost: 'hover:bg-accent hover:text-accent-foreground',
				destructive: 'bg-destructive text-white hover:bg-destructive/90',
				outline: 'border border-input bg-transparent hover:bg-accent hover:text-accent-foreground',
				link: 'text-primary underline-offset-4 hover:underline'
			},
			size: {
				default: 'h-9 px-4 py-2',
				sm: 'h-8 rounded-md px-3 text-xs',
				lg: 'h-10 px-6',
				icon: 'size-9'
			}
		},
		defaultVariants: { variant: 'default', size: 'default' }
	});
	export type ButtonVariant = VariantProps<typeof buttonVariants>['variant'];
	export type ButtonSize = VariantProps<typeof buttonVariants>['size'];
</script>

<script lang="ts">
	import { cn } from '$lib/utils';
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';

	let {
		class: className,
		variant = 'default',
		size = 'default',
		children,
		...rest
	}: HTMLButtonAttributes & { variant?: ButtonVariant; size?: ButtonSize; children?: Snippet } =
		$props();
</script>

<button class={cn(buttonVariants({ variant, size }), className)} {...rest}>
	{@render children?.()}
</button>
