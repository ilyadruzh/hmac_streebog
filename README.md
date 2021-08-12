# ZoKrates

zokrates compile -i zokrates_stdlib/stdlib/streebog.zok
zokrates compute-witness -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5

записываем свидетеля в zok

zokrates compile -i hmacstreebog.zok

./zokrates setup
./zokrates export-verifier

./zokrates compute-witness -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5

./zokrates generate-proof
