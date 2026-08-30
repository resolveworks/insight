#!/usr/bin/env bash
# Fetch test PDFs into ./samples: the papers from Ilya Sutskever's
# "30 under 30" reading list (dzyim/ilya-sutskever-recommended-reading).
# The repo only links the papers, so this materializes the PDF portion.
# Blogs and course notes on the list are skipped (no PDF exists).
#
# Usage: scripts/fetch-samples.sh [dest-dir]   (default: <repo>/samples)
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST=${1:-$REPO_ROOT/samples}
mkdir -p "$DEST"
cd "$DEST"

fetch() { # fetch <filename> <url> [sleep-after]
	local name=$1 url=$2 delay=${3:-0}
	if [[ -s $name ]]; then echo "SKIP  $name (exists)"; return; fi
	echo "GET   $name"
	curl -sSL --fail --max-time 60 -o "$name" "$url" && sleep "$delay"
}

# arXiv papers (1s delay to be polite)
fetch zaremba-2014-rnn-regularization.pdf         https://arxiv.org/pdf/1409.2329 1
fetch vinyals-2015-pointer-networks.pdf           https://arxiv.org/pdf/1506.03134 1
fetch vinyals-2016-order-matters.pdf              https://arxiv.org/pdf/1511.06391 1
fetch huang-2019-gpipe.pdf                        https://arxiv.org/pdf/1811.06965 1
fetch he-2015-resnet.pdf                          https://arxiv.org/pdf/1512.03385 1
fetch yu-2016-dilated-convolutions.pdf            https://arxiv.org/pdf/1511.07122 1
fetch gilmer-2017-neural-message-passing.pdf      https://arxiv.org/pdf/1704.01212 1
fetch vaswani-2017-attention-is-all-you-need.pdf  https://arxiv.org/pdf/1706.03762 1
fetch bahdanau-2014-seq2seq-attention.pdf         https://arxiv.org/pdf/1409.0473 1
fetch he-2016-identity-mappings.pdf               https://arxiv.org/pdf/1603.05027 1
fetch santoro-2017-relational-reasoning.pdf       https://arxiv.org/pdf/1706.01427 1
fetch chen-2016-variational-lossy-autoencoder.pdf https://arxiv.org/pdf/1611.02731 1
fetch santoro-2018-relational-rnn.pdf             https://arxiv.org/pdf/1806.01893 1
fetch aaronson-2014-coffee-automaton.pdf          https://arxiv.org/pdf/1405.6903 1
fetch graves-2014-neural-turing-machines.pdf      https://arxiv.org/pdf/1410.5401 1
fetch amodei-2015-deep-speech-2.pdf               https://arxiv.org/pdf/1512.02595 1
fetch kaplan-2020-scaling-laws.pdf                https://arxiv.org/pdf/2001.08361 1
fetch grunwald-2004-mdl-tutorial.pdf              https://arxiv.org/pdf/math/0406077 1

# Non-arXiv PDFs
fetch hinton-1993-description-length-weights.pdf https://www.cs.toronto.edu/~hinton/absps/colt93.pdf
fetch krizhevsky-2012-alexnet.pdf                https://proceedings.neurips.cc/paper/2012/file/c399862d3b9d6b76c8436e924a68c45b-Paper.pdf
fetch legg-2006-machine-super-intelligence.pdf   http://www.vetta.org/documents/Machine_Super_Intelligence.pdf
fetch shen-kolmogorov-complexity.pdf             https://www.lirmm.fr/~ashen/kolmbook-eng.pdf

echo; echo "=== results ==="
for f in *.pdf; do
	# Non-PDF results are kept on purpose: broken files exercise Insight's error paths.
	file -b "$f" | grep -qi 'pdf document' && echo "ok    $f ($(du -h "$f" | cut -f1))" || echo "WARN  $f — not a valid PDF (kept for error-path testing)"
done | sort
