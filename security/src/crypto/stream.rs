//! Chiffrement en flux par blocs, pour les gros fichiers (vidéos, sauvegardes).
//!
//! Construction STREAM (Hoang, Reyhanitabar, Rogaway, Vizár, 2015), variante « BE32 » de
//! `aead-stream` : le nonce de chaque bloc est `préfixe aléatoire (19) ‖ compteur (u32) ‖
//! drapeau final (1)`. Chaque bloc est authentifié séparément avec les données associées de
//! l'en-tête, ce qui détecte :
//!
//! - un bit modifié (étiquette Poly1305) ;
//! - un bloc supprimé, dupliqué ou réordonné (compteur dans le nonce) ;
//! - une troncature (le dernier bloc porte le drapeau final ; un flux qui s'arrête sans lui
//!   est refusé avec [`CryptoError::Truncated`]) ;
//! - des octets ajoutés après le bloc final.
//!
//! Découpage : tous les blocs sauf le dernier contiennent exactement `chunk_size` octets de
//! texte clair ; le dernier en contient strictement moins (éventuellement zéro). Un bloc
//! chiffré de taille pleine n'est donc jamais final, ce qui rend le découpage non ambigu.
//!
//! **Lecture progressive.** Le texte clair est livré bloc par bloc, chaque bloc étant
//! authentifié avant d'être livré ; mais la complétude du flux n'est établie qu'à la fin.
//! Un appelant qui reçoit une erreur doit jeter tout ce qu'il a déjà lu.

use std::io::{self, Read, Write};

use aead_stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::XChaCha20Poly1305;

use super::CryptoError;
use super::kdf::KdfParams;
use super::key::{DataKey, TAG_LEN, random_array};
use super::slot::KeySlot;
use super::vault::{PayloadKind, PayloadParams, STREAM_NONCE_PREFIX_LEN, Unlock, VaultHeader};

/// Taille de bloc par défaut : 64 Kio de texte clair.
pub const DEFAULT_CHUNK_SIZE: u32 = 64 * 1024;
/// Taille de bloc minimale acceptée.
pub const MIN_CHUNK_SIZE: u32 = 16;
/// Taille de bloc maximale acceptée (borne la mémoire allouée à la lecture).
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

fn check_chunk_size(chunk_size: u32) -> Result<usize, CryptoError> {
    if (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
        usize::try_from(chunk_size).map_err(|_| CryptoError::InvalidChunkSize(chunk_size))
    } else {
        Err(CryptoError::InvalidChunkSize(chunk_size))
    }
}

/// Chiffre un flux : implémente [`Write`] ; **[`StreamEncryptor::finish`] est obligatoire**.
///
/// Un chiffreur abandonné sans `finish` produit un flux sans bloc final, refusé à la lecture
/// ([`CryptoError::Truncated`]) : une écriture interrompue ne passe jamais pour complète.
pub struct StreamEncryptor<W: Write> {
    writer: W,
    stream: EncryptorBE32<XChaCha20Poly1305>,
    header: VaultHeader,
    aad: Vec<u8>,
    chunk_size: usize,
    buffer: Vec<u8>,
    failed: bool,
}

impl<W: Write> StreamEncryptor<W> {
    /// Chiffreur sous une clé de données (coffre à clé externe), blocs de 64 Kio.
    ///
    /// # Errors
    /// Aléa indisponible ; écriture de l'en-tête impossible.
    pub fn new(key: &DataKey, writer: W) -> Result<Self, CryptoError> {
        Self::with_options(key, None, DEFAULT_CHUNK_SIZE, writer)
    }

    /// Chiffreur sous une nouvelle clé de données protégée par `password` (coffre autonome).
    ///
    /// # Errors
    /// Aléa indisponible ; échec d'Argon2id ; écriture de l'en-tête impossible.
    pub fn with_password(
        password: &[u8],
        params: KdfParams,
        writer: W,
    ) -> Result<Self, CryptoError> {
        let (slot, key) = KeySlot::create(password, params)?;
        Self::with_options(&key, Some(&slot), DEFAULT_CHUNK_SIZE, writer)
    }

    /// Chiffreur complet : `slot` (s'il est fourni) est embarqué dans l'en-tête et doit
    /// protéger `key` ; `chunk_size` est la taille de texte clair des blocs.
    ///
    /// # Errors
    /// [`CryptoError::InvalidChunkSize`], [`CryptoError::KeyMismatch`] ; aléa indisponible ;
    /// écriture de l'en-tête impossible.
    pub fn with_options(
        key: &DataKey,
        slot: Option<&KeySlot>,
        chunk_size: u32,
        mut writer: W,
    ) -> Result<Self, CryptoError> {
        let chunk_len = check_chunk_size(chunk_size)?;
        let nonce_prefix = random_array::<STREAM_NONCE_PREFIX_LEN>()?;
        let header = VaultHeader::new(
            key,
            slot,
            PayloadParams::Stream {
                chunk_size,
                nonce_prefix,
            },
        )?;
        writer.write_all(&header.encode())?;
        Ok(Self {
            writer,
            stream: EncryptorBE32::from_aead(key.cipher(), &nonce_prefix.into()),
            aad: header.payload_aad(),
            header,
            chunk_size: chunk_len,
            buffer: Vec::with_capacity(chunk_len + TAG_LEN),
            failed: false,
        })
    }

    /// En-tête écrit au début du flux.
    pub const fn header(&self) -> &VaultHeader {
        &self.header
    }

    /// Chiffre et écrit un bloc plein (non final).
    fn emit_full_chunk(&mut self) -> Result<(), CryptoError> {
        self.stream
            .encrypt_next_in_place(&self.aad, &mut self.buffer)
            .map_err(|_| CryptoError::StreamTooLong)?;
        self.writer.write_all(&self.buffer)?;
        self.buffer.clear();
        Ok(())
    }

    /// Chiffre le bloc final, vide le tampon et rend l'écrivain sous-jacent.
    ///
    /// # Errors
    /// Écriture impossible, ou chiffreur déjà en échec.
    pub fn finish(self) -> Result<W, CryptoError> {
        let Self {
            mut writer,
            stream,
            aad,
            mut buffer,
            failed,
            ..
        } = self;
        if failed {
            return Err(CryptoError::Malformed(
                "chiffreur interrompu par une erreur précédente",
            ));
        }
        stream
            .encrypt_last_in_place(&aad, &mut buffer)
            .map_err(|_| CryptoError::StreamTooLong)?;
        writer.write_all(&buffer)?;
        writer.flush()?;
        Ok(writer)
    }
}

impl<W: Write> Write for StreamEncryptor<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.failed {
            return Err(
                CryptoError::Malformed("chiffreur interrompu par une erreur précédente").into(),
            );
        }
        let mut rest = data;
        while !rest.is_empty() {
            let take = (self.chunk_size - self.buffer.len()).min(rest.len());
            let (head, tail) = rest.split_at(take);
            self.buffer.extend_from_slice(head);
            rest = tail;
            if self.buffer.len() == self.chunk_size
                && let Err(e) = self.emit_full_chunk()
            {
                self.failed = true;
                return Err(e.into());
            }
        }
        Ok(data.len())
    }

    /// Transmet les blocs complets déjà écrits ; le bloc en cours reste en mémoire jusqu'à
    /// ce qu'il soit plein ou jusqu'à [`StreamEncryptor::finish`].
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<W: Write> std::fmt::Debug for StreamEncryptor<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamEncryptor")
            .field("header", &self.header)
            .field("chunk_size", &self.chunk_size)
            .field("buffered", &self.buffer.len())
            .finish_non_exhaustive()
    }
}

/// État de lecture d'un [`StreamDecryptor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadState {
    Reading,
    Finished,
    Failed,
}

/// Déchiffre un flux : implémente [`Read`] et ne livre que des blocs authentifiés.
///
/// La fin du flux ([`Read::read`] renvoyant 0) n'est atteinte qu'après vérification du bloc
/// final ; un flux tronqué produit une erreur [`io::ErrorKind::UnexpectedEof`] portant
/// [`CryptoError::Truncated`] (récupérable avec [`CryptoError::from_io`]).
pub struct StreamDecryptor<R: Read> {
    reader: R,
    stream: Option<DecryptorBE32<XChaCha20Poly1305>>,
    header: VaultHeader,
    aad: Vec<u8>,
    chunk_size: usize,
    buffer: Vec<u8>,
    position: usize,
    state: ReadState,
}

impl<R: Read> StreamDecryptor<R> {
    /// Lit l'en-tête et prépare le déchiffrement.
    ///
    /// # Errors
    /// En-tête invalide ; [`CryptoError::WrongPayloadKind`] ; [`CryptoError::WrongPassword`] ;
    /// [`CryptoError::KeyMismatch`] ; [`CryptoError::NoPasswordSlot`].
    pub fn new(unlock: Unlock<'_>, mut reader: R) -> Result<Self, CryptoError> {
        let header = VaultHeader::read_from(&mut reader)?;
        header.expect_kind(PayloadKind::Stream)?;
        let PayloadParams::Stream {
            chunk_size,
            nonce_prefix,
        } = *header.payload()
        else {
            return Err(CryptoError::Malformed("paramètres de contenu incohérents"));
        };
        let chunk_len = check_chunk_size(chunk_size)?;
        let cipher = header.cipher(unlock)?;
        Ok(Self {
            reader,
            stream: Some(DecryptorBE32::from_aead(cipher, &nonce_prefix.into())),
            aad: header.payload_aad(),
            header,
            chunk_size: chunk_len,
            buffer: Vec::with_capacity(chunk_len + TAG_LEN),
            position: 0,
            state: ReadState::Reading,
        })
    }

    /// En-tête lu au début du flux.
    pub const fn header(&self) -> &VaultHeader {
        &self.header
    }

    /// Bloc suivant de texte clair authentifié, ou `None` après le bloc final.
    ///
    /// # Errors
    /// [`CryptoError::AuthenticationFailed`] (bit modifié, bloc supprimé ou réordonné, octets
    /// ajoutés), [`CryptoError::Truncated`], entrée-sortie. Après une erreur, toute lecture
    /// suivante échoue.
    pub fn next_chunk(&mut self) -> Result<Option<&[u8]>, CryptoError> {
        match self.state {
            ReadState::Finished => return Ok(None),
            ReadState::Failed => {
                return Err(CryptoError::Malformed(
                    "lecture interrompue par une erreur précédente",
                ));
            }
            ReadState::Reading => {}
        }
        match self.decrypt_next() {
            Ok(()) => {
                self.position = self.buffer.len();
                Ok(Some(&self.buffer))
            }
            Err(e) => {
                self.state = ReadState::Failed;
                self.buffer.clear();
                Err(e)
            }
        }
    }

    /// Lit et déchiffre le bloc suivant dans `self.buffer`.
    fn decrypt_next(&mut self) -> Result<(), CryptoError> {
        let full = self.chunk_size + TAG_LEN;
        self.buffer.clear();
        self.position = 0;
        (&mut self.reader)
            .take(full as u64)
            .read_to_end(&mut self.buffer)?;
        let len = self.buffer.len();
        if len == full {
            let stream = self.stream.as_mut().ok_or(CryptoError::Truncated)?;
            stream
                .decrypt_next_in_place(&self.aad, &mut self.buffer)
                .map_err(|_| CryptoError::AuthenticationFailed)
        } else if len < TAG_LEN {
            // Fin des données sans bloc final (ou bloc final amputé de son étiquette).
            Err(CryptoError::Truncated)
        } else {
            let stream = self.stream.take().ok_or(CryptoError::Truncated)?;
            stream
                .decrypt_last_in_place(&self.aad, &mut self.buffer)
                .map_err(|_| CryptoError::AuthenticationFailed)?;
            self.state = ReadState::Finished;
            Ok(())
        }
    }

    /// Rend le lecteur sous-jacent.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R: Read> Read for StreamDecryptor<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.position < self.buffer.len() {
                let available = &self.buffer[self.position..];
                let n = available.len().min(out.len());
                out[..n].copy_from_slice(&available[..n]);
                self.position += n;
                return Ok(n);
            }
            if out.is_empty() {
                return Ok(0);
            }
            match self.state {
                ReadState::Finished => return Ok(0),
                ReadState::Failed | ReadState::Reading => {
                    self.next_chunk().map_err(io::Error::from)?;
                    // `next_chunk` a placé `position` en fin de bloc : on relit depuis le début.
                    self.position = 0;
                }
            }
        }
    }
}

impl<R: Read> std::fmt::Debug for StreamDecryptor<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamDecryptor")
            .field("header", &self.header)
            .field("chunk_size", &self.chunk_size)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Chiffre tout `reader` vers `writer` sous une clé de données ; renvoie la taille du clair.
///
/// # Errors
/// Voir [`StreamEncryptor::with_options`] ; entrée-sortie.
pub fn encrypt_stream<R: Read, W: Write>(
    key: &DataKey,
    slot: Option<&KeySlot>,
    mut reader: R,
    writer: W,
) -> Result<(u64, W), CryptoError> {
    let mut encryptor = StreamEncryptor::with_options(key, slot, DEFAULT_CHUNK_SIZE, writer)?;
    let copied = io::copy(&mut reader, &mut encryptor).map_err(CryptoError::from_io)?;
    Ok((copied, encryptor.finish()?))
}

/// Déchiffre tout le flux de `reader` vers `writer` ; renvoie la taille du clair.
///
/// Le texte clair est écrit au fil de l'eau : en cas d'erreur, ce qui a déjà été écrit doit
/// être détruit par l'appelant.
///
/// # Errors
/// Voir [`StreamDecryptor::next_chunk`].
pub fn decrypt_stream<R: Read, W: Write>(
    unlock: Unlock<'_>,
    reader: R,
    mut writer: W,
) -> Result<u64, CryptoError> {
    let mut decryptor = StreamDecryptor::new(unlock, reader)?;
    let mut total = 0u64;
    while let Some(chunk) = decryptor.next_chunk()? {
        writer.write_all(chunk)?;
        total += chunk.len() as u64;
    }
    writer.flush()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::super::slot::tests::fast;
    use super::super::vault::rotate_password_in_memory;
    use super::*;

    const CHUNK: u32 = 32;
    const FULL: usize = CHUNK as usize + TAG_LEN;

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn encrypt(key: &DataKey, data: &[u8]) -> Vec<u8> {
        let mut enc = StreamEncryptor::with_options(key, None, CHUNK, Vec::new()).unwrap();
        // Écritures de tailles irrégulières pour exercer le découpage.
        for part in data.chunks(7) {
            enc.write_all(part).unwrap();
        }
        enc.finish().unwrap()
    }

    fn decrypt(key: &DataKey, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut out = Vec::new();
        decrypt_stream(Unlock::Key(key), sealed, &mut out)?;
        Ok(out)
    }

    fn header_len(sealed: &[u8]) -> usize {
        VaultHeader::parse(sealed).unwrap().1
    }

    #[test]
    fn round_trip_all_boundary_sizes() {
        let key = DataKey::generate().unwrap();
        for len in [0, 1, 31, 32, 33, 63, 64, 65, 200] {
            let data = sample(len);
            let sealed = encrypt(&key, &data);
            // Blocs pleins non finaux + un bloc final strictement plus court.
            let chunks = len / CHUNK as usize + 1;
            assert_eq!(
                sealed.len(),
                header_len(&sealed) + len + chunks * TAG_LEN,
                "taille {len}"
            );
            assert_eq!(decrypt(&key, &sealed).unwrap(), data, "taille {len}");

            // Lecture via `Read`, par petites tranches.
            let mut reader = StreamDecryptor::new(Unlock::Key(&key), sealed.as_slice()).unwrap();
            let mut out = Vec::new();
            let mut buf = [0u8; 5];
            loop {
                let n = reader.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
            assert_eq!(out, data);
        }
    }

    #[test]
    fn password_mode_and_rotation() {
        let data = sample(1000);
        let mut enc =
            StreamEncryptor::with_password("vidéo".as_bytes(), fast(), Vec::new()).unwrap();
        enc.write_all(&data).unwrap();
        let sealed = enc.finish().unwrap();
        let mut out = Vec::new();
        decrypt_stream(
            Unlock::Password(b"vid\xc3\xa9o"),
            sealed.as_slice(),
            &mut out,
        )
        .unwrap();
        assert_eq!(out, data);

        let rotated =
            rotate_password_in_memory(&sealed, "vidéo".as_bytes(), b"neuf", fast()).unwrap();
        assert_eq!(
            &rotated[header_len(&rotated)..],
            &sealed[header_len(&sealed)..]
        );
        let mut out = Vec::new();
        decrypt_stream(Unlock::Password(b"neuf"), rotated.as_slice(), &mut out).unwrap();
        assert_eq!(out, data);
        assert!(matches!(
            StreamDecryptor::new(Unlock::Password(b"vid\xc3\xa9o"), rotated.as_slice()),
            Err(CryptoError::WrongPassword)
        ));
    }

    #[test]
    fn flipped_bit_anywhere_is_detected() {
        let key = DataKey::generate().unwrap();
        let sealed = encrypt(&key, &sample(100));
        for i in 0..sealed.len() {
            let mut tampered = sealed.clone();
            tampered[i] ^= 0x04;
            assert!(
                decrypt(&key, &tampered).is_err(),
                "octet {i} modifié sans être détecté"
            );
        }
    }

    #[test]
    fn removed_reordered_or_duplicated_chunks_are_detected() {
        let key = DataKey::generate().unwrap();
        let sealed = encrypt(&key, &sample(100)); // 3 blocs pleins + 1 bloc final de 4 octets
        let h = header_len(&sealed);
        let chunk = |i: usize| &sealed[h + i * FULL..(h + (i + 1) * FULL).min(sealed.len())];
        let assemble = |order: &[usize]| {
            let mut out = sealed[..h].to_vec();
            for &i in order {
                out.extend_from_slice(chunk(i));
            }
            out
        };
        assert_eq!(
            decrypt(&key, &assemble(&[0, 1, 2, 3])).unwrap(),
            sample(100)
        );
        for order in [
            &[0, 2, 3][..],       // bloc du milieu supprimé
            &[1, 0, 2, 3][..],    // blocs permutés
            &[0, 1, 1, 2, 3][..], // bloc dupliqué
            &[0, 1, 3, 2][..],    // bloc final déplacé
        ] {
            assert!(
                matches!(
                    decrypt(&key, &assemble(order)),
                    Err(CryptoError::AuthenticationFailed)
                ),
                "ordre {order:?} accepté"
            );
        }
    }

    #[test]
    fn truncation_is_detected() {
        let key = DataKey::generate().unwrap();
        let data = sample(96); // exactement 3 blocs pleins + bloc final vide
        let sealed = encrypt(&key, &data);
        let h = header_len(&sealed);
        assert_eq!(sealed.len(), h + 3 * FULL + TAG_LEN);

        // Coupure à une frontière de bloc : le bloc final manque.
        for blocks in 0..=3 {
            let cut = &sealed[..h + blocks * FULL];
            assert!(
                matches!(decrypt(&key, cut), Err(CryptoError::Truncated)),
                "{blocks} blocs sans bloc final"
            );
        }
        // Coupure au milieu d'un bloc.
        for cut in [h + 1, h + FULL - 1, h + FULL + TAG_LEN, sealed.len() - 1] {
            assert!(decrypt(&key, &sealed[..cut]).is_err(), "coupure à {cut}");
        }
        // Erreur de lecture typée via `Read`.
        let mut reader = StreamDecryptor::new(Unlock::Key(&key), &sealed[..h + FULL]).unwrap();
        let err = reader.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(matches!(CryptoError::from_io(err), CryptoError::Truncated));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let key = DataKey::generate().unwrap();
        for len in [10, 96] {
            let mut sealed = encrypt(&key, &sample(len));
            sealed.push(0);
            assert!(decrypt(&key, &sealed).is_err(), "octet ajouté après {len}");
            sealed.extend_from_slice(&[0u8; FULL]);
            assert!(decrypt(&key, &sealed).is_err(), "bloc ajouté après {len}");
        }
    }

    #[test]
    fn unfinished_encryptor_yields_truncated_stream() {
        let key = DataKey::generate().unwrap();
        let mut sink = Vec::new();
        {
            let mut enc = StreamEncryptor::with_options(&key, None, CHUNK, &mut sink).unwrap();
            enc.write_all(&sample(80)).unwrap();
            // Abandonné sans `finish` (plantage, disque plein…).
        }
        assert!(matches!(decrypt(&key, &sink), Err(CryptoError::Truncated)));
    }

    #[test]
    fn wrong_key_kind_and_chunk_size() {
        let key = DataKey::generate().unwrap();
        let other = DataKey::generate().unwrap();
        let sealed = encrypt(&key, b"abc");
        assert!(matches!(
            StreamDecryptor::new(Unlock::Key(&other), sealed.as_slice()),
            Err(CryptoError::KeyMismatch)
        ));
        let blob = super::super::blob::seal(&key, b"abc").unwrap();
        assert!(matches!(
            StreamDecryptor::new(Unlock::Key(&key), blob.as_slice()),
            Err(CryptoError::WrongPayloadKind { .. })
        ));
        assert!(matches!(
            StreamEncryptor::with_options(&key, None, MIN_CHUNK_SIZE - 1, Vec::new()),
            Err(CryptoError::InvalidChunkSize(_))
        ));
        assert!(StreamEncryptor::with_options(&key, None, MAX_CHUNK_SIZE + 1, Vec::new()).is_err());

        // Taille de bloc forgée dans l'en-tête : refusée avant toute allocation.
        let mut forged = sealed.clone();
        let at = 8 + 2 + 1 + 1 + 16;
        forged[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            StreamDecryptor::new(Unlock::Key(&key), forged.as_slice()),
            Err(CryptoError::InvalidChunkSize(u32::MAX))
        ));
    }

    #[test]
    fn encrypt_stream_helper() {
        let key = DataKey::generate().unwrap();
        let data = sample(200_000);
        let (n, sealed) = encrypt_stream(&key, None, data.as_slice(), Vec::new()).unwrap();
        assert_eq!(n, 200_000);
        assert_eq!(decrypt(&key, &sealed).unwrap(), data);
    }
}
