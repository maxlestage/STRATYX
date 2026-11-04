//! Sauvegardes chiffrées : un ensemble de fichiers devient un seul fichier coffre.
//!
//! L'archive est écrite dans un coffre en flux ([`StreamEncryptor`]), en général autonome
//! (emplacement de clé embarqué, déverrouillable par mot de passe sur une autre machine).
//! Format du texte clair :
//!
//! ```text
//! « STRXBAK\0 », version u16
//! pour chaque fichier : 0x01, longueur du chemin u16, chemin UTF-8, taille u64,
//!                       contenu, SHA-256 du contenu (32 octets)
//! fin : 0x00, nombre de fichiers u64, total des octets u64
//! ```
//!
//! **Intégrité.** Chaque bloc du flux est authentifié (bit modifié, bloc supprimé ou
//! réordonné, troncature : refus) ; chaque fichier porte en plus son empreinte SHA-256,
//! vérifiée à la restauration ; le pied d'archive recense fichiers et octets.
//!
//! **Restauration sûre.** Les chemins sont validés (ni absolus, ni `..`, ni noms réservés
//! de Windows) ; les fichiers sont écrits dans un répertoire de transit, renommé en
//! destination seulement si toute l'archive est vérifiée, et supprimé sinon.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{
    CryptoError, DataKey, KdfParams, KeySlot, StreamDecryptor, StreamEncryptor, Unlock,
};

/// Signature du texte clair d'une archive de sauvegarde.
pub const BACKUP_MAGIC: [u8; 8] = *b"STRXBAK\0";
/// Version du format d'archive.
pub const BACKUP_VERSION: u16 = 1;
/// Longueur maximale d'un chemin d'archive, en octets.
pub const MAX_PATH_LEN: usize = 1024;

const TAG_FILE: u8 = 1;
const TAG_END: u8 = 0;

/// Erreur de sauvegarde ou de restauration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackupError {
    /// Chiffrement, mot de passe, altération ou troncature du coffre.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// Erreur d'entrée-sortie (lecture des sources, écriture des fichiers restaurés).
    #[error("erreur d'entrée-sortie : {0}")]
    Io(#[from] io::Error),
    /// Chemin d'archive refusé.
    #[error("chemin d'archive refusé : « {0} »")]
    InvalidPath(String),
    /// Deux fichiers avec le même chemin.
    #[error("chemin présent deux fois dans l'archive : « {0} »")]
    DuplicatePath(String),
    /// Une source a changé de taille pendant la sauvegarde.
    #[error(
        "le fichier « {path} » a changé pendant la sauvegarde \
         ({expected} octets attendus, {actual} lus)"
    )]
    SourceChanged {
        /// Chemin d'archive.
        path: String,
        /// Taille annoncée.
        expected: u64,
        /// Taille lue.
        actual: u64,
    },
    /// La destination de la restauration existe déjà.
    #[error("la destination « {} » existe déjà : restauration refusée", .0.display())]
    DestinationExists(PathBuf),
    /// Archive authentique mais incohérente.
    #[error("archive incohérente : {0}")]
    Corrupted(&'static str),
    /// Empreinte d'un fichier incorrecte.
    #[error("empreinte SHA-256 incorrecte pour « {0} »")]
    DigestMismatch(String),
    /// Le rédacteur a échoué précédemment ; l'archive est inutilisable.
    #[error("sauvegarde interrompue par une erreur précédente")]
    Poisoned,
}

impl BackupError {
    /// Convertit une erreur de lecture de l'archive (éventuellement cryptographique).
    fn from_read(error: io::Error) -> Self {
        match CryptoError::from_io(error) {
            CryptoError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Self::Corrupted("archive tronquée avant son pied")
            }
            CryptoError::Io(e) => Self::Io(e),
            other => Self::Crypto(other),
        }
    }
}

/// Bilan d'une sauvegarde ou d'une restauration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSummary {
    /// Nombre de fichiers.
    pub files: u64,
    /// Total des octets de contenu.
    pub bytes: u64,
}

/// Valide un chemin d'archive : relatif, séparateur `/`, sans composant vide, `.` ou `..`,
/// sans `\`, `:`, caractère de contrôle, nom réservé de Windows ni composant finissant par
/// un point ou une espace.
///
/// # Errors
/// [`BackupError::InvalidPath`].
pub fn validate_archive_path(path: &str) -> Result<(), BackupError> {
    let invalid = || BackupError::InvalidPath(path.to_owned());
    if path.is_empty()
        || path.len() > MAX_PATH_LEN
        || path.starts_with('/')
        || path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':'))
    {
        return Err(invalid());
    }
    for component in path.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with('.')
            || component.ends_with(' ')
            || reserved
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Écrivain qui calcule l'empreinte SHA-256 de ce qui le traverse.
struct HashingWriter<'a, W: Write> {
    inner: &'a mut W,
    hasher: Sha256,
}

impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(data)?;
        self.hasher.update(&data[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Lecteur qui calcule l'empreinte SHA-256 de ce qui le traverse.
struct HashingReader<R: Read> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(out)?;
        self.hasher.update(&out[..n]);
        Ok(n)
    }
}

/// Rédacteur d'archive de sauvegarde chiffrée. **[`BackupWriter::finish`] est obligatoire** :
/// sans lui, l'archive est tronquée et refusée à la restauration.
pub struct BackupWriter<W: Write> {
    stream: StreamEncryptor<W>,
    paths: BTreeSet<String>,
    summary: BackupSummary,
    poisoned: bool,
}

impl<W: Write> BackupWriter<W> {
    /// Archive autonome protégée par `password` (nouvelle clé de données, emplacement embarqué).
    ///
    /// # Errors
    /// Échec d'Argon2id, aléa indisponible, écriture impossible.
    pub fn new(password: &[u8], params: KdfParams, output: W) -> Result<Self, BackupError> {
        Self::start(StreamEncryptor::with_password(password, params, output)?)
    }

    /// Archive chiffrée par une clé de données ; `slot`, s'il est fourni, est embarqué.
    ///
    /// # Errors
    /// [`CryptoError::KeyMismatch`] ; aléa indisponible ; écriture impossible.
    pub fn with_key(key: &DataKey, slot: Option<&KeySlot>, output: W) -> Result<Self, BackupError> {
        Self::start(StreamEncryptor::with_options(
            key,
            slot,
            crate::crypto::stream::DEFAULT_CHUNK_SIZE,
            output,
        )?)
    }

    fn start(mut stream: StreamEncryptor<W>) -> Result<Self, BackupError> {
        stream.write_all(&BACKUP_MAGIC)?;
        stream.write_all(&BACKUP_VERSION.to_be_bytes())?;
        Ok(Self {
            stream,
            paths: BTreeSet::new(),
            summary: BackupSummary::default(),
            poisoned: false,
        })
    }

    /// Ajoute un fichier dont le contenu est en mémoire.
    ///
    /// # Errors
    /// Chemin invalide ou en double ; écriture impossible.
    pub fn add_bytes(&mut self, path: &str, data: &[u8]) -> Result<(), BackupError> {
        self.add_reader(path, data.len() as u64, data)
    }

    /// Ajoute un fichier de `size` octets lus depuis `reader` (lu en flux, sans tout charger).
    ///
    /// # Errors
    /// Chemin invalide ou en double ; [`BackupError::SourceChanged`] si `reader` fournit
    /// moins de `size` octets (l'archive devient alors inutilisable) ; écriture impossible.
    pub fn add_reader<R: Read>(
        &mut self,
        path: &str,
        size: u64,
        reader: R,
    ) -> Result<(), BackupError> {
        if self.poisoned {
            return Err(BackupError::Poisoned);
        }
        validate_archive_path(path)?;
        if self.paths.contains(path) {
            return Err(BackupError::DuplicatePath(path.to_owned()));
        }
        let result = self.write_entry(path, size, reader);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn write_entry<R: Read>(
        &mut self,
        path: &str,
        size: u64,
        reader: R,
    ) -> Result<(), BackupError> {
        let path_len =
            u16::try_from(path.len()).map_err(|_| BackupError::InvalidPath(path.to_owned()))?;
        self.stream.write_all(&[TAG_FILE])?;
        self.stream.write_all(&path_len.to_be_bytes())?;
        self.stream.write_all(path.as_bytes())?;
        self.stream.write_all(&size.to_be_bytes())?;
        let mut sink = HashingWriter {
            inner: &mut self.stream,
            hasher: Sha256::new(),
        };
        let copied = io::copy(&mut reader.take(size), &mut sink).map_err(BackupError::from_read)?;
        let digest = sink.hasher.finalize();
        if copied != size {
            return Err(BackupError::SourceChanged {
                path: path.to_owned(),
                expected: size,
                actual: copied,
            });
        }
        self.stream.write_all(&digest)?;
        self.paths.insert(path.to_owned());
        self.summary.files += 1;
        self.summary.bytes += size;
        Ok(())
    }

    /// Ajoute un fichier du disque sous le chemin d'archive `path`.
    ///
    /// # Errors
    /// Voir [`BackupWriter::add_reader`] ; [`BackupError::SourceChanged`] si le fichier
    /// grandit pendant la lecture.
    pub fn add_file(&mut self, path: &str, source: &Path) -> Result<(), BackupError> {
        let mut file = fs::File::open(source)?;
        let size = file.metadata()?.len();
        self.add_reader(path, size, &mut file)?;
        let mut probe = [0u8; 1];
        if file.read(&mut probe)? != 0 {
            self.poisoned = true;
            return Err(BackupError::SourceChanged {
                path: path.to_owned(),
                expected: size,
                actual: size + 1,
            });
        }
        Ok(())
    }

    /// Ajoute récursivement les fichiers ordinaires de `root` (liens symboliques ignorés),
    /// dans l'ordre alphabétique, sous des chemins relatifs à `root` ; renvoie leur nombre.
    ///
    /// # Errors
    /// Nom de fichier non UTF-8 ou refusé ; erreurs de [`BackupWriter::add_file`].
    pub fn add_directory(&mut self, root: &Path) -> Result<u64, BackupError> {
        let mut files = Vec::new();
        collect_files(root, "", &mut files)?;
        for (archive_path, source) in &files {
            self.add_file(archive_path, source)?;
        }
        Ok(files.len() as u64)
    }

    /// Bilan provisoire.
    #[must_use]
    pub const fn summary(&self) -> BackupSummary {
        self.summary
    }

    /// Écrit le pied d'archive et le bloc final ; rend l'écrivain et le bilan.
    ///
    /// # Errors
    /// [`BackupError::Poisoned`] après une erreur ; écriture impossible.
    pub fn finish(mut self) -> Result<(W, BackupSummary), BackupError> {
        if self.poisoned {
            return Err(BackupError::Poisoned);
        }
        self.stream.write_all(&[TAG_END])?;
        self.stream.write_all(&self.summary.files.to_be_bytes())?;
        self.stream.write_all(&self.summary.bytes.to_be_bytes())?;
        Ok((self.stream.finish()?, self.summary))
    }
}

impl<W: Write> std::fmt::Debug for BackupWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupWriter")
            .field("summary", &self.summary)
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), BackupError> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| BackupError::InvalidPath(name.to_string_lossy().into_owned()))?;
        let archive_path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&entry.path(), &archive_path, out)?;
        } else if file_type.is_file() {
            validate_archive_path(&archive_path)?;
            out.push((archive_path, entry.path()));
        }
    }
    Ok(())
}

/// Fichier restauré en mémoire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredFile {
    /// Chemin d'archive.
    pub path: String,
    /// Contenu.
    pub data: Vec<u8>,
}

fn read_exact<const N: usize, R: Read>(reader: &mut R) -> Result<[u8; N], BackupError> {
    let mut out = [0u8; N];
    reader
        .read_exact(&mut out)
        .map_err(BackupError::from_read)?;
    Ok(out)
}

/// Récepteur d'un fichier d'archive : chemin validé, taille, contenu à lire entièrement.
type EntrySink<'a> = dyn FnMut(&str, u64, &mut dyn Read) -> Result<(), BackupError> + 'a;

/// Parcourt une archive ; `sink` reçoit chaque fichier (chemin validé, taille, contenu à lire
/// entièrement) et l'empreinte est vérifiée après lui.
fn read_archive<R: Read>(
    input: R,
    unlock: Unlock<'_>,
    sink: &mut EntrySink<'_>,
) -> Result<BackupSummary, BackupError> {
    let mut stream = StreamDecryptor::new(unlock, input)?;
    if read_exact::<8, _>(&mut stream)? != BACKUP_MAGIC {
        return Err(BackupError::Corrupted("signature d'archive absente"));
    }
    let version = u16::from_be_bytes(read_exact(&mut stream)?);
    if version != BACKUP_VERSION {
        return Err(BackupError::Crypto(CryptoError::UnsupportedVersion(
            version,
        )));
    }
    let mut paths = BTreeSet::new();
    let mut summary = BackupSummary::default();
    loop {
        match read_exact::<1, _>(&mut stream)?[0] {
            TAG_FILE => {}
            TAG_END => break,
            _ => return Err(BackupError::Corrupted("type d'entrée inconnu")),
        }
        let path_len = usize::from(u16::from_be_bytes(read_exact(&mut stream)?));
        if path_len > MAX_PATH_LEN {
            return Err(BackupError::Corrupted("chemin trop long"));
        }
        let mut raw_path = vec![0u8; path_len];
        stream
            .read_exact(&mut raw_path)
            .map_err(BackupError::from_read)?;
        let path =
            String::from_utf8(raw_path).map_err(|_| BackupError::Corrupted("chemin non UTF-8"))?;
        validate_archive_path(&path)?;
        if !paths.insert(path.clone()) {
            return Err(BackupError::DuplicatePath(path));
        }
        let size = u64::from_be_bytes(read_exact(&mut stream)?);

        let mut content = HashingReader {
            inner: (&mut stream).take(size),
            hasher: Sha256::new(),
        };
        sink(&path, size, &mut content)?;
        // Le récepteur doit tout lire ; on vérifie qu'il ne reste rien.
        if io::copy(&mut content, &mut io::sink()).map_err(BackupError::from_read)? != 0
            || content.inner.limit() != 0
        {
            return Err(BackupError::Corrupted("contenu de fichier incomplet"));
        }
        let computed: [u8; 32] = content.hasher.finalize().into();
        if read_exact::<32, _>(&mut stream)? != computed {
            return Err(BackupError::DigestMismatch(path));
        }
        summary.files += 1;
        summary.bytes += size;
    }
    let files = u64::from_be_bytes(read_exact(&mut stream)?);
    let bytes = u64::from_be_bytes(read_exact(&mut stream)?);
    if files != summary.files || bytes != summary.bytes {
        return Err(BackupError::Corrupted("pied d'archive incohérent"));
    }
    // La fin du flux n'est atteinte qu'après authentification du bloc final.
    let mut probe = [0u8; 1];
    if stream.read(&mut probe).map_err(BackupError::from_read)? != 0 {
        return Err(BackupError::Corrupted("données après le pied d'archive"));
    }
    Ok(summary)
}

/// Vérifie une archive complète (authenticité, empreintes, pied) sans rien écrire.
///
/// # Errors
/// Voir [`BackupError`].
pub fn verify_backup<R: Read>(input: R, unlock: Unlock<'_>) -> Result<BackupSummary, BackupError> {
    read_archive(input, unlock, &mut |_, _, content| {
        io::copy(content, &mut io::sink()).map_err(BackupError::from_read)?;
        Ok(())
    })
}

/// Restaure une archive en mémoire (petites archives, tests). Rien n'est renvoyé si une
/// vérification échoue.
///
/// # Errors
/// Voir [`BackupError`].
pub fn restore_to_memory<R: Read>(
    input: R,
    unlock: Unlock<'_>,
) -> Result<Vec<RestoredFile>, BackupError> {
    let mut files = Vec::new();
    read_archive(input, unlock, &mut |path, _, content| {
        let mut data = Vec::new();
        content
            .read_to_end(&mut data)
            .map_err(BackupError::from_read)?;
        files.push(RestoredFile {
            path: path.to_owned(),
            data,
        });
        Ok(())
    })?;
    Ok(files)
}

/// Restaure une archive dans `destination`, qui ne doit pas exister.
///
/// Les fichiers sont écrits dans un répertoire de transit voisin, renommé en `destination`
/// seulement si toute l'archive est vérifiée ; en cas d'échec, il est supprimé et
/// `destination` n'est pas créée.
///
/// # Errors
/// [`BackupError::DestinationExists`] ; voir [`BackupError`].
pub fn restore_to_directory<R: Read>(
    input: R,
    unlock: Unlock<'_>,
    destination: &Path,
) -> Result<BackupSummary, BackupError> {
    if destination.exists() {
        return Err(BackupError::DestinationExists(destination.to_owned()));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| BackupError::InvalidPath(destination.display().to_string()))?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut suffix = [0u8; 8];
    getrandom::fill(&mut suffix).map_err(CryptoError::from)?;
    let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let staging = parent.join(format!(".{}.restauration-{suffix}", name.to_string_lossy()));
    fs::create_dir(&staging)?;

    let result = read_archive(input, unlock, &mut |path, _, content| {
        let target = path
            .split('/')
            .fold(staging.clone(), |acc, part| acc.join(part));
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        io::copy(content, &mut file).map_err(BackupError::from_read)?;
        file.sync_all()?;
        Ok(())
    })
    .and_then(|summary| {
        fs::rename(&staging, destination)?;
        Ok(summary)
    });
    if result.is_err() {
        // Meilleur effort : le répertoire de transit ne contient que des données à jeter.
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{KdfParams, VaultHeader};

    fn fast() -> KdfParams {
        KdfParams::new(64, 1, 1).unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let mut suffix = [0u8; 6];
        getrandom::fill(&mut suffix).unwrap();
        let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("stratyx-security-{name}-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_archive(password: &[u8]) -> Vec<u8> {
        let mut writer = BackupWriter::new(password, fast(), Vec::new()).unwrap();
        writer
            .add_bytes("mains/2026-11-04.json", b"[{\"id\":1}]")
            .unwrap();
        writer
            .add_bytes("videos/session-1.stxv", &vec![42u8; 200_000])
            .unwrap();
        writer.add_bytes("vide.txt", b"").unwrap();
        let (bytes, summary) = writer.finish().unwrap();
        assert_eq!(
            summary,
            BackupSummary {
                files: 3,
                bytes: 200_010
            }
        );
        bytes
    }

    #[test]
    fn memory_round_trip() {
        let archive = sample_archive(b"sauvegarde");
        let files = restore_to_memory(archive.as_slice(), Unlock::Password(b"sauvegarde")).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["mains/2026-11-04.json", "videos/session-1.stxv", "vide.txt"]
        );
        assert_eq!(files[1].data, vec![42u8; 200_000]);
        assert_eq!(
            verify_backup(archive.as_slice(), Unlock::Password(b"sauvegarde"))
                .unwrap()
                .files,
            3
        );
        assert!(matches!(
            verify_backup(archive.as_slice(), Unlock::Password(b"autre")),
            Err(BackupError::Crypto(CryptoError::WrongPassword))
        ));
    }

    #[test]
    fn tampering_and_truncation_are_detected() {
        let archive = sample_archive(b"pw");
        let (_, header_len) = VaultHeader::parse(&archive).unwrap();
        for at in [
            header_len,
            header_len + 100,
            archive.len() / 2,
            archive.len() - 1,
        ] {
            let mut tampered = archive.clone();
            tampered[at] ^= 0x10;
            assert!(
                matches!(
                    verify_backup(tampered.as_slice(), Unlock::Password(b"pw")),
                    Err(BackupError::Crypto(CryptoError::AuthenticationFailed))
                ),
                "octet {at}"
            );
        }
        for cut in [header_len, archive.len() - 17, archive.len() - 1] {
            assert!(
                verify_backup(&archive[..cut], Unlock::Password(b"pw")).is_err(),
                "coupure {cut}"
            );
        }
    }

    #[test]
    fn digest_mismatch_and_footer_are_checked() {
        // Archive authentique mais mal formée (écrite à la main sous la même clé) : les
        // contrôles internes la refusent.
        let key = DataKey::generate().unwrap();
        let build = |digest_ok: bool, footer_files: u64| {
            let mut enc = StreamEncryptor::new(&key, Vec::new()).unwrap();
            enc.write_all(&BACKUP_MAGIC).unwrap();
            enc.write_all(&BACKUP_VERSION.to_be_bytes()).unwrap();
            enc.write_all(&[TAG_FILE]).unwrap();
            enc.write_all(&5u16.to_be_bytes()).unwrap();
            enc.write_all(b"a.txt").unwrap();
            enc.write_all(&3u64.to_be_bytes()).unwrap();
            enc.write_all(b"abc").unwrap();
            let mut digest: [u8; 32] = Sha256::digest(b"abc").into();
            if !digest_ok {
                digest[0] ^= 1;
            }
            enc.write_all(&digest).unwrap();
            enc.write_all(&[TAG_END]).unwrap();
            enc.write_all(&footer_files.to_be_bytes()).unwrap();
            enc.write_all(&3u64.to_be_bytes()).unwrap();
            enc.finish().unwrap()
        };
        assert!(restore_to_memory(build(true, 1).as_slice(), Unlock::Key(&key)).is_ok());
        assert!(matches!(
            restore_to_memory(build(false, 1).as_slice(), Unlock::Key(&key)),
            Err(BackupError::DigestMismatch(p)) if p == "a.txt"
        ));
        assert!(matches!(
            restore_to_memory(build(true, 2).as_slice(), Unlock::Key(&key)),
            Err(BackupError::Corrupted(_))
        ));
    }

    #[test]
    fn path_validation() {
        for ok in ["a", "dossier/fichier.json", "é/ü.txt", ".cache/x"] {
            validate_archive_path(ok).unwrap();
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "./a",
            "a\\b",
            "C:/x",
            "a/NUL",
            "con.txt",
            "com1",
            "a/b.",
            "a/b ",
            "a\nb",
        ] {
            assert!(validate_archive_path(bad).is_err(), "{bad:?} accepté");
        }
        let mut writer =
            BackupWriter::with_key(&DataKey::generate().unwrap(), None, Vec::new()).unwrap();
        assert!(matches!(
            writer.add_bytes("../evasion", b""),
            Err(BackupError::InvalidPath(_))
        ));
        writer.add_bytes("a", b"1").unwrap();
        assert!(matches!(
            writer.add_bytes("a", b"2"),
            Err(BackupError::DuplicatePath(_))
        ));
        // Ces refus n'empoisonnent pas l'archive.
        writer.finish().unwrap();
    }

    #[test]
    fn short_source_poisons_the_archive() {
        let mut writer = BackupWriter::new(b"pw", fast(), Vec::new()).unwrap();
        let err = writer.add_reader("f", 10, &b"abc"[..]).unwrap_err();
        assert!(matches!(
            err,
            BackupError::SourceChanged {
                expected: 10,
                actual: 3,
                ..
            }
        ));
        assert!(matches!(
            writer.add_bytes("g", b""),
            Err(BackupError::Poisoned)
        ));
        assert!(matches!(writer.finish(), Err(BackupError::Poisoned)));
    }

    #[test]
    fn directory_round_trip() {
        let root = scratch("dir");
        let source = root.join("source");
        fs::create_dir_all(source.join("mains/2026")).unwrap();
        fs::create_dir_all(source.join("videos")).unwrap();
        fs::write(source.join("mains/2026/m1.json"), b"{\"main\":1}").unwrap();
        fs::write(source.join("videos/v1.stxv"), vec![7u8; 150_000]).unwrap();
        fs::write(source.join("config.json"), b"{}").unwrap();

        let archive_path = root.join("sauvegarde.stxvault");
        let mut writer =
            BackupWriter::new(b"pw", fast(), fs::File::create(&archive_path).unwrap()).unwrap();
        assert_eq!(writer.add_directory(&source).unwrap(), 3);
        let (_, summary) = writer.finish().unwrap();
        assert_eq!(summary.files, 3);

        let restored = root.join("restauration");
        let report = restore_to_directory(
            fs::File::open(&archive_path).unwrap(),
            Unlock::Password(b"pw"),
            &restored,
        )
        .unwrap();
        assert_eq!(report, summary);
        for rel in ["mains/2026/m1.json", "videos/v1.stxv", "config.json"] {
            assert_eq!(
                fs::read(restored.join(rel)).unwrap(),
                fs::read(source.join(rel)).unwrap()
            );
        }

        // Destination existante : refus.
        assert!(matches!(
            restore_to_directory(
                fs::File::open(&archive_path).unwrap(),
                Unlock::Password(b"pw"),
                &restored
            ),
            Err(BackupError::DestinationExists(_))
        ));

        // Archive altérée : rien n'est créé, le transit est nettoyé.
        let mut bytes = fs::read(&archive_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let failed = root.join("echec");
        assert!(restore_to_directory(bytes.as_slice(), Unlock::Password(b"pw"), &failed).is_err());
        assert!(!failed.exists());
        let leftovers: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("restauration-"))
            .collect();
        assert!(leftovers.is_empty(), "répertoire de transit non nettoyé");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rgpd_export_into_encrypted_backup() {
        use crate::gdpr::{AuditLog, DataCategory, DataSubject, InMemoryStore, handle_export};
        let subject = DataSubject::user("moi");
        let mut store = InMemoryStore::new();
        store.insert(
            DataCategory::Statistics,
            "s",
            0,
            [subject.clone()],
            "application/json",
            b"{\"roi\":0.12}".to_vec(),
        );
        let export = handle_export(&store, &subject, &mut AuditLog::new(), 0).unwrap();

        let mut writer = BackupWriter::new(b"export", fast(), Vec::new()).unwrap();
        for (path, data) in export.files() {
            writer.add_bytes(&path, &data).unwrap();
        }
        let (archive, _) = writer.finish().unwrap();
        let files = restore_to_memory(archive.as_slice(), Unlock::Password(b"export")).unwrap();
        assert_eq!(files[0].path, "manifeste.json");
        assert_eq!(files[1].data, b"{\"roi\":0.12}");
    }
}
