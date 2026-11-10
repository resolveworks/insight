/** Formats an ISO timestamp for display, in the user's locale. */
export function formatTimestamp(iso: string): string {
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: 'medium',
		timeStyle: 'short',
	}).format(new Date(iso));
}

/** Shortens an id to its first `length` characters, ellipsised. */
export function shortenId(id: string, length = 8): string {
	return id.length <= length ? id : `${id.slice(0, length)}…`;
}
