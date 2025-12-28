// Highlight download buttons matching user's platform
(function () {
	const ua = navigator.userAgent.toLowerCase();
	let platform = null;
	if (ua.includes('mac')) platform = 'macos';
	else if (ua.includes('win')) platform = 'windows';
	else if (ua.includes('linux')) platform = 'linux';

	if (platform) {
		document
			.querySelectorAll('.download-btn[data-platform="' + platform + '"]')
			.forEach((btn) => {
				btn.classList.add('primary');
			});
	}
})();
