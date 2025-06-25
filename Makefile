# Makefile for HyperChain whitepaper
WHITEPAPER = hyperchain-whitepaper.pdf
MKDIR = docs/whitepaper

all: $(WHITEPAPER)

$(WHITEPAPER):
	pandoc $(MKDIR)/hyperchain-whitepaper.md \
	--from markdown+implicit_figures \
	--include-in-header header.tex \
	--pdf-engine=lualatex \
	--resource-path=.:$(MKDIR)/assets \
	-V geometry:margin=1in \
	-V documentclass=extarticle \
	-o $@

clean:
	rm -f $(WHITEPAPER) part*

.PHONY: all clean
