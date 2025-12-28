// Fetch latest release from GitHub and populate download links
(function () {
	const REPO = 'resolveworks/insight';

	const container = document.getElementById('download-links');
	if (!container) return;

	// Platform detection
	function detectPlatform() {
		const ua = navigator.userAgent.toLowerCase();
		if (ua.includes('mac')) return 'macos';
		if (ua.includes('win')) return 'windows';
		if (ua.includes('linux')) return 'linux';
		return null;
	}

	// Match assets to platforms
	function categorizeAssets(assets) {
		const platforms = {
			macos: {
				label: 'macOS',
				patterns: [/\.dmg$/i, /darwin.*\.tar\.gz$/i, /macos/i],
				asset: null,
			},
			windows: {
				label: 'Windows',
				patterns: [/\.msi$/i, /\.exe$/i, /windows/i],
				asset: null,
			},
			linux: {
				label: 'Linux',
				patterns: [/\.AppImage$/i, /\.deb$/i, /linux.*\.tar\.gz$/i],
				asset: null,
			},
		};

		for (const asset of assets) {
			for (const platform of Object.values(platforms)) {
				if (
					!platform.asset &&
					platform.patterns.some((p) => p.test(asset.name))
				) {
					platform.asset = asset;
					break;
				}
			}
		}

		return platforms;
	}

	function render(release, platforms) {
		const detected = detectPlatform();
		let html = `<p class="release-version">Latest version: <strong>${release.tag_name}</strong></p>`;
		html += '<div class="download-buttons">';

		for (const [key, platform] of Object.entries(platforms)) {
			if (platform.asset) {
				const isPrimary = key === detected;
				const btnClass = isPrimary ? 'download-btn primary' : 'download-btn';
				html += `<a href="${platform.asset.browser_download_url}" class="${btnClass}">${platform.label}</a>`;
			}
		}

		html += '</div>';
		container.innerHTML = html;
	}

	fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
		.then((res) => (res.ok ? res.json() : Promise.reject()))
		.then((release) => {
			const platforms = categorizeAssets(release.assets || []);
			if (Object.values(platforms).some((p) => p.asset)) {
				render(release, platforms);
			}
		})
		.catch(() => {});
})();
