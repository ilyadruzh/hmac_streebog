# ZoKrates

zokrates compile -i streebog.zok -o streebog_constr --ztf

zokrates compute-witness -i streebog_constr -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5

zokrates compute-witness --verbose -a 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15

записываем свидетеля в zok

zokrates compile -i stdlib/hmac/streebog2.zok

zokrates setup

zokrates export-verifier

zokrates compute-witness -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5

zokrates generate-proof

