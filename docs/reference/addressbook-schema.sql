CREATE TABLE _SqliteDatabaseProperties (key TEXT, value TEXT, UNIQUE(key));
CREATE TABLE ABPersonSearchKey (person_id INTEGER, SearchKey BLOB, NameOnlySearchKey BLOB, UNIQUE(person_id));
CREATE TABLE ABGroupMembers (UID INTEGER PRIMARY KEY, group_id INTEGER, member_type INTEGER, member_id INTEGER, UNIQUE(group_id, member_type, member_id));
CREATE TABLE ABMultiValue (UID INTEGER PRIMARY KEY, record_id INTEGER, property INTEGER, identifier INTEGER, label INTEGER, value TEXT, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), UNIQUE(guid));
CREATE TABLE ABMultiValueLabel (value TEXT, UNIQUE(value));
CREATE TABLE ABMultiValueEntry (parent_id INTEGER, key INTEGER, value TEXT, UNIQUE(parent_id, key));
CREATE TABLE ABMultiValueEntryKey (value TEXT, UNIQUE(value));
CREATE TABLE ABPhoneLastFour (multivalue_id INTEGER PRIMARY KEY, value TEXT);
CREATE TABLE ABPersonMultiValueDeletes (record_id INTEGER, property_id INTEGER, identifier INTEGER, sequence_number INTEGER, record_uid TEXT, identifier_uid TEXT);
CREATE TABLE ClientCursor (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, store_id INTEGER, latest_consumed_sequence_number INTEGER, latest_consumed_timestamp REAL, client_identifier TEXT, UNIQUE(client_identifier, store_id));
CREATE TABLE sqlite_sequence(name,seq);
CREATE TABLE ClientCursorConsumed (store_id INTEGER, consumed_entity_class INTEGER, consumed_entity_id INTEGER, consumed_change_id INTEGER, sequence_number INTEGER, client_identifier TEXT);
CREATE TABLE ClientSequence (sequence_number INTEGER,client_identifier TEXT,save_timestamp REAL );
CREATE TABLE LimitedAccess (BundleID TEXT NOT NULL, guid TEXT NOT NULL, SequenceNumber INTEGER, IsActive INTEGER, PRIMARY KEY (BundleID, guid));
CREATE TABLE sqlite_stat1(tbl,idx,stat);
CREATE TABLE ABStore (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, Name TEXT, ExternalIdentifier TEXT, Type INTEGER, ConstraintsPath TEXT, ExternalModificationTag TEXT, ExternalSyncTag TEXT, StoreInternalIdentifier TEXT, AccountID INTEGER DEFAULT -1,Enabled INTEGER DEFAULT 1, SyncData BLOB, MeIdentifier INTEGER DEFAULT -1, Capabilities INTEGER DEFAULT 0, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), LastSyncDate TEXT, ProviderIdentifier TEXT, ProviderMetadata BLOB, UNIQUE(StoreInternalIdentifier), UNIQUE(guid));
CREATE TABLE ABAccount (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, AccountIdentifier TEXT, Flags INTEGER, DefaultSourceID INTEGER, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), UNIQUE(guid));
CREATE TABLE ABGroup (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, Name TEXT, ExternalIdentifier TEXT, StoreID INTEGER, ExternalModificationTag TEXT, ExternalRepresentation BLOB, ExternalUUID TEXT, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), UNIQUE(guid));
CREATE TABLE ABGroupChanges (record INTEGER, type INTEGER, sequence_number INTEGER, ExternalIdentifier TEXT, StoreID INTEGER, guid TEXT);
CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, First TEXT, Last TEXT, Middle TEXT, FirstPhonetic TEXT, MiddlePhonetic TEXT, LastPhonetic TEXT, Organization TEXT, Department TEXT, Note TEXT, Kind INTEGER, Birthday TEXT, JobTitle TEXT, Nickname TEXT, Prefix TEXT, Suffix TEXT, FirstSort TEXT, LastSort TEXT, CreationDate INTEGER, ModificationDate INTEGER, CompositeNameFallback TEXT, ExternalIdentifier TEXT, ExternalModificationTag TEXT, ExternalUUID TEXT, StoreID INTEGER, DisplayName TEXT, ExternalRepresentation BLOB, FirstSortSection TEXT, LastSortSection TEXT, FirstSortLanguageIndex INTEGER DEFAULT 2147483647, LastSortLanguageIndex INTEGER DEFAULT 2147483647, PersonLink INTEGER DEFAULT -1, ImageURI TEXT, IsPreferredName INTEGER DEFAULT 1, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), PhonemeData TEXT, AlternateBirthday TEXT, MapsData TEXT, FirstPronunciation TEXT, MiddlePronunciation TEXT, LastPronunciation TEXT, OrganizationPhonetic TEXT, OrganizationPronunciation TEXT, PreviousFamilyName TEXT, PreferredLikenessSource TEXT, PreferredPersonaIdentifier TEXT, PreferredChannel TEXT, DowntimeWhitelist TEXT, ImageType TEXT, ImageHash BLOB, MemojiMetadata BLOB, Wallpaper BLOB, DisplayFlags INTEGER, WatchWallpaperImageData BLOB, WallpaperMetadata BLOB, ImageBackgroundColorsData BLOB, SensitiveContentConfiguration BLOB, WallpaperURI TEXT, ImageSyncFailedTime TEXT, WallpaperSyncFailedTime TEXT, UNIQUE(guid));
CREATE TABLE ABPersonChanges (record INTEGER, type INTEGER, sequence_number INTEGER, Image INTEGER, ExternalIdentifier TEXT, StoreID INTEGER, PersonLink INTEGER, guid TEXT, Changes INTEGER, LinkToPersonUUID TEXT, PersonLinkUUID TEXT);
CREATE TABLE ABPersonLink (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, PreferredImagePersonID INTEGER DEFAULT -1, PreferredNamePersonID INTEGER DEFAULT -1, guid TEXT NOT NULL DEFAULT (ab_generate_guid()), UNIQUE(guid));
CREATE TABLE FirstSortSectionCount(LanguageIndex, Section TEXT, StoreID, number, UNIQUE(LanguageIndex, Section, StoreID));
CREATE TABLE LastSortSectionCount(LanguageIndex, Section TEXT, StoreID, number, UNIQUE(LanguageIndex, Section, StoreID));
CREATE TABLE FirstSortSectionCountTotal(LanguageIndex, Section TEXT, Type INT, number, UNIQUE(LanguageIndex, Section, Type));
CREATE TABLE LastSortSectionCountTotal(LanguageIndex, Section TEXT, Type INT, number, UNIQUE(LanguageIndex, Section, Type));
CREATE TABLE IF NOT EXISTS 'ABPersonFullTextSearch_content'(docid INTEGER PRIMARY KEY, 'c0First', 'c1Last', 'c2Middle', 'c3FirstPhonetic', 'c4MiddlePhonetic', 'c5LastPhonetic', 'c6Organization', 'c7OrganizationPhonetic', 'c8Department', 'c9Note', 'c10Birthday', 'c11JobTitle', 'c12Nickname', 'c13Prefix', 'c14Suffix', 'c15DisplayName', 'c16Phone', 'c17Email', 'c18Address', 'c19SocialProfile', 'c20URL', 'c21RelatedNames', 'c22IM', 'c23Date', 'c24SupplementalCompositeNameTerms');
CREATE TABLE IF NOT EXISTS 'ABPersonFullTextSearch_segments'(blockid INTEGER PRIMARY KEY, block BLOB);
CREATE TABLE IF NOT EXISTS 'ABPersonFullTextSearch_segdir'(level INTEGER,idx INTEGER,start_block INTEGER,leaves_end_block INTEGER,end_block INTEGER,root BLOB,PRIMARY KEY(level, idx));
CREATE TABLE IF NOT EXISTS 'ABPersonFullTextSearch_docsize'(docid INTEGER PRIMARY KEY, size BLOB);
CREATE TABLE IF NOT EXISTS 'ABPersonFullTextSearch_stat'(id INTEGER PRIMARY KEY, value BLOB);
CREATE TABLE IF NOT EXISTS 'ABPersonSmartDialerFullTextSearch_content'(docid INTEGER PRIMARY KEY, 'c0FirstEncoding', 'c1MiddleEncoding', 'c2LastEncoding', 'c3OrganizationEncoding', 'c4NicknameEncoding', 'c5PhoneLastFourDigits');
CREATE TABLE IF NOT EXISTS 'ABPersonSmartDialerFullTextSearch_segments'(blockid INTEGER PRIMARY KEY, block BLOB);
CREATE TABLE IF NOT EXISTS 'ABPersonSmartDialerFullTextSearch_segdir'(level INTEGER,idx INTEGER,start_block INTEGER,leaves_end_block INTEGER,end_block INTEGER,root BLOB,PRIMARY KEY(level, idx));
CREATE TABLE IF NOT EXISTS 'ABPersonSmartDialerFullTextSearch_docsize'(docid INTEGER PRIMARY KEY, size BLOB);
CREATE TABLE IF NOT EXISTS 'ABPersonSmartDialerFullTextSearch_stat'(id INTEGER PRIMARY KEY, value BLOB);
CREATE INDEX ClientCursorClientIdentifier ON ClientCursor(client_identifier, store_id);
CREATE INDEX ClientCursorConsumedClientIdentifier ON ClientCursorConsumed(client_identifier, store_id);
CREATE INDEX ABMultiValueRecordIDIndex on ABMultiValue(record_id);
CREATE INDEX ABMultiValueLabelIndex ON ABMultiValue(label);
CREATE INDEX ABMultiValueGUIDIndex ON ABMultiValue(guid);
CREATE INDEX ABMultiValuePropertyValueIndex ON ABMultiValue(property, value COLLATE NOCASE);
CREATE INDEX ABMultiValueEntryKeyIndex ON ABMultiValueEntry(key);
CREATE INDEX ABPhoneLastFourIndex ON ABPhoneLastFour(value);
CREATE INDEX ClientSequenceClientIdentifier ON ClientSequence(client_identifier);
CREATE INDEX ABMultiValueEntryParentIndex ON ABMultiValueEntry(parent_id);
CREATE INDEX ABLimitedAccessBundleIndex ON LimitedAccess(BundleID);
CREATE INDEX ABLimitedAccessGUIDIndex ON LimitedAccess(guid);
CREATE INDEX ABLimitedAccessSequenceNumber ON LimitedAccess(SequenceNumber);
CREATE INDEX ABLimitedAccesssActiveIndex ON LimitedAccess(IsActive);
CREATE INDEX ABFirstSortIndex on ABPerson(FirstSortLanguageIndex, FirstSortSection, FirstSort);
CREATE INDEX ABLastSortIndex on ABPerson(LastSortLanguageIndex, LastSortSection, LastSort);
CREATE INDEX ABFirstSortStoreIndex on ABPerson(StoreID, FirstSortLanguageIndex, FirstSortSection, FirstSort);
CREATE INDEX ABLastSortStoreIndex on ABPerson(StoreID, LastSortLanguageIndex, LastSortSection, LastSort);
CREATE INDEX ABPersonPersonLinkIndex on ABPerson(PersonLink);
CREATE INDEX ABPreferredFirstSortIndex on ABPerson(IsPreferredName, FirstSortLanguageIndex, FirstSortSection, FirstSort);
CREATE INDEX ABPreferredLastSortIndex on ABPerson(IsPreferredName, LastSortLanguageIndex, LastSortSection, LastSort);
CREATE INDEX ABPersonExternalUUIDIndex on ABPerson(ExternalUUID);
CREATE INDEX ABPersonExternalIdentifierIndex on ABPerson(ExternalIdentifier);
CREATE TRIGGER delete_phone_last_four AFTER DELETE ON ABMultiValue
BEGIN
DELETE FROM ABPhoneLastFour WHERE multivalue_id = OLD.UID;
END;
CREATE TRIGGER delete_clientcursor_consumed AFTER DELETE ON ClientCursor 
    BEGIN 
        DELETE FROM ClientCursorConsumed WHERE client_identifier = OLD.client_identifier AND store_id = OLD.store_id; 
    END;
CREATE TRIGGER DeletePersonLink AFTER DELETE ON ABPerson BEGIN DELETE FROM ABPersonLink WHERE (ROWID = OLD.PersonLink AND (SELECT count() FROM ABPerson WHERE PersonLink = OLD.PersonLink) < 2); END;
CREATE TRIGGER UpdatePersonLink AFTER UPDATE OF PersonLink ON ABPerson BEGIN DELETE FROM ABPersonLink WHERE (ROWID = OLD.PersonLink AND (SELECT count() FROM ABPerson WHERE PersonLink = OLD.PersonLink) < 2); END;
CREATE TRIGGER UpdatePersonLinkUponPersonUnlink AFTER UPDATE OF PersonLink ON ABPerson WHEN OLD.PersonLink != -1 AND NEW.PersonLink = -1 BEGIN UPDATE ABPersonLink SET PreferredNamePersonID = (SELECT ROWID FROM ABPerson abp WHERE abp.PersonLink = OLD.PersonLink ORDER BY abp.ROWID LIMIT 1) WHERE (ROWID = OLD.PersonLink AND PreferredNamePersonID = OLD.ROWID); END;
CREATE TRIGGER UpdatePersonUponLinkDeletion AFTER DELETE ON ABPersonLink BEGIN     UPDATE ABPerson SET PersonLink = -1, IsPreferredName = 1 WHERE PersonLink = OLD.ROWID; END;
CREATE TRIGGER UpdatePersonUponLinkInsertion AFTER INSERT ON ABPersonLink 
BEGIN 
    UPDATE ABPerson SET IsPreferredName = ab_update_value_from_trigger(ROWID = NEW.PreferredNamePersonID, 'IsPreferredName', ROWID) WHERE PersonLink = NEW.ROWID; 
END;
CREATE TRIGGER UpdatePersonUponLinkUpdate AFTER UPDATE ON ABPersonLink 
BEGIN 
    UPDATE ABPerson SET IsPreferredName = ab_update_value_from_trigger(ROWID = NEW.PreferredNamePersonID, 'IsPreferredName', ROWID) WHERE PersonLink = NEW.ROWID; 
END;
CREATE TRIGGER NamePreferredPersonEditTrigger AFTER UPDATE OF Prefix, First, Middle, Last, Suffix, Organization, Kind ON ABPerson WHEN ((OLD.Prefix IS NOT NEW.Prefix) OR (OLD.First IS NOT NEW.First) OR (OLD.Middle IS NOT NEW.Middle) OR (OLD.Last IS NOT NEW.Last) OR (OLD.Suffix IS NOT NEW.Suffix) OR (OLD.Organization IS NOT NEW.Organization) OR (OLD.Kind IS NOT NEW.Kind)) BEGIN UPDATE ABPersonLink SET PreferredNamePersonID = OLD.ROWID WHERE (OLD.PersonLink = ROWID); END;
CREATE TRIGGER NamePreferredPersonInsertTrigger AFTER INSERT ON ABPerson BEGIN UPDATE ABPersonLink SET PreferredNamePersonID = NEW.ROWID WHERE (ROWID = NEW.PersonLink AND PreferredNamePersonID = -1); END;
CREATE TRIGGER NamePreferredPersonDeleteTrigger AFTER DELETE ON ABPerson BEGIN UPDATE ABPersonLink SET PreferredNamePersonID = (SELECT ROWID FROM ABPerson abp WHERE abp.PersonLink = OLD.PersonLink ORDER BY abp.ROWID LIMIT 1) WHERE (ROWID = OLD.PersonLink AND PreferredNamePersonID = OLD.ROWID); END;
CREATE TRIGGER UpdatePersonLinkField AFTER UPDATE OF PersonLink ON ABPerson 
WHEN OLD.PersonLink != NEW.PersonLink 
BEGIN 
    UPDATE ABPerson 
    SET IsPreferredName = ab_update_value_from_trigger((NEW.PersonLink = -1 OR ROWID = (SELECT PreferredNamePersonID FROM ABPersonLink abl WHERE abl.ROWID = NEW.PersonLink)), 'IsPreferredName', ROWID) 
    WHERE ROWID = NEW.ROWID; 
END;
CREATE TRIGGER InitializePersonLinkField AFTER INSERT ON ABPerson 
BEGIN 
    UPDATE ABPerson 
    SET IsPreferredName = ab_update_value_from_trigger((NEW.PersonLink = -1 OR ROWID = (SELECT PreferredNamePersonID FROM ABPersonLink abl WHERE abl.ROWID = NEW.PersonLink)), 'IsPreferredName', ROWID) 
    WHERE ROWID = NEW.ROWID; 
END;
CREATE TRIGGER DeleteSearchKey AFTER DELETE ON ABPerson BEGIN DELETE FROM ABPersonSearchKey WHERE person_id = OLD.ROWID; END;
CREATE TRIGGER RemoveDeletedContactsFromLimited AFTER DELETE ON ABPerson 
BEGIN 
    DELETE FROM LimitedAccess 
    WHERE Old.guid = guid    ; 
END;
CREATE TRIGGER UpdateLinkedContactsInLimited AFTER DELETE ON ABPersonLink 
BEGIN 
    INSERT OR IGNORE INTO LimitedAccess (BundleId, guid, SequenceNumber, IsActive) 
        SELECT la.BundleId, abp.guid, la.SequenceNumber, la.IsActive FROM LimitedAccess la, ABPerson abp 
        WHERE la.guid = OLD.guid AND abp.PersonLink = OLD.ROWID; 
    DELETE FROM LimitedAccess WHERE guid = OLD.guid; 
END;
CREATE TRIGGER FirstSortSectionInsert AFTER INSERT ON ABPerson BEGIN INSERT OR REPLACE INTO FirstSortSectionCount VALUES(NEW.FirstSortLanguageIndex, IFNULL(NEW.FirstSortSection, ''), NEW.StoreID, 1 + IFNULL((SELECT number from FirstSortSectionCount WHERE LanguageIndex = NEW.FirstSortLanguageIndex AND Section = IFNULL(NEW.FirstSortSection, '') AND StoreID = NEW.StoreID), 0)); END;
CREATE TRIGGER FirstSortSectionUpdate AFTER UPDATE ON ABPerson BEGIN INSERT OR REPLACE INTO FirstSortSectionCount VALUES(OLD.FirstSortLanguageIndex, IFNULL(OLD.FirstSortSection, ''), OLD.StoreID, (SELECT number from FirstSortSectionCount WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND StoreID = OLD.StoreID) - 1); DELETE FROM FirstSortSectionCount WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND StoreID = OLD.StoreID AND number = 0; INSERT OR REPLACE INTO FirstSortSectionCount VALUES(NEW.FirstSortLanguageIndex, IFNULL(NEW.FirstSortSection, ''), NEW.StoreID, 1 + IFNULL((SELECT number from FirstSortSectionCount WHERE LanguageIndex = NEW.FirstSortLanguageIndex AND Section = IFNULL(NEW.FirstSortSection, '') AND StoreID = NEW.StoreID), 0)); END;
CREATE TRIGGER FirstSortSectionDelete AFTER DELETE ON ABPerson BEGIN INSERT OR REPLACE INTO FirstSortSectionCount VALUES(OLD.FirstSortLanguageIndex, IFNULL(OLD.FirstSortSection, ''), OLD.StoreID, (SELECT number from FirstSortSectionCount WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND StoreID = OLD.StoreID) - 1); DELETE FROM FirstSortSectionCount WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND StoreID = OLD.StoreID AND number = 0; END;
CREATE TRIGGER LastSortSectionInsert AFTER INSERT ON ABPerson BEGIN INSERT OR REPLACE INTO LastSortSectionCount VALUES(NEW.LastSortLanguageIndex, IFNULL(NEW.LastSortSection, ''), NEW.StoreID, 1 + IFNULL((SELECT number from LastSortSectionCount WHERE LanguageIndex = NEW.LastSortLanguageIndex AND Section = IFNULL(NEW.LastSortSection, '') AND StoreID = NEW.StoreID), 0)); END;
CREATE TRIGGER LastSortSectionUpdate AFTER UPDATE ON ABPerson BEGIN INSERT OR REPLACE INTO LastSortSectionCount VALUES(OLD.LastSortLanguageIndex, IFNULL(OLD.LastSortSection, ''), OLD.StoreID, (SELECT number from LastSortSectionCount WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND StoreID = OLD.StoreID) - 1); DELETE FROM LastSortSectionCount WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND StoreID = OLD.StoreID AND number = 0; INSERT OR REPLACE INTO LastSortSectionCount VALUES(NEW.LastSortLanguageIndex, IFNULL(NEW.LastSortSection, ''), NEW.StoreID, 1 + IFNULL((SELECT number from LastSortSectionCount WHERE LanguageIndex = NEW.LastSortLanguageIndex AND Section = IFNULL(NEW.LastSortSection, '') AND StoreID = NEW.StoreID), 0)); END;
CREATE TRIGGER LastSortSectionDelete AFTER DELETE ON ABPerson BEGIN INSERT OR REPLACE INTO LastSortSectionCount VALUES(OLD.LastSortLanguageIndex, IFNULL(OLD.LastSortSection, ''), OLD.StoreID, (SELECT number from LastSortSectionCount WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND StoreID = OLD.StoreID) - 1); DELETE FROM LastSortSectionCount WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND StoreID = OLD.StoreID AND number = 0; END;
CREATE TRIGGER FirstSortSectionTotalDelete AFTER DELETE ON ABPerson 
WHEN OLD.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO FirstSortSectionCountTotal 
    VALUES( 
        OLD.FirstSortLanguageIndex, 
        IFNULL(OLD.FirstSortSection, ''), 
        0, 
        (SELECT number FROM FirstSortSectionCountTotal WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND Type = 0) - 1 
    ); 
    
    DELETE FROM FirstSortSectionCountTotal 
    WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND 
        Section = IFNULL(OLD.FirstSortSection, '') AND 
        Type = 0 AND 
        number <= 0; 
END;
CREATE TRIGGER LastSortSectionTotalDelete AFTER DELETE ON ABPerson 
WHEN OLD.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO LastSortSectionCountTotal 
    VALUES( 
        OLD.LastSortLanguageIndex, 
        IFNULL(OLD.LastSortSection, ''), 
        0, 
        (SELECT number FROM LastSortSectionCountTotal WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND Type = 0) - 1 
    ); 
    
    DELETE FROM LastSortSectionCountTotal 
    WHERE LanguageIndex = OLD.LastSortLanguageIndex AND 
        Section = IFNULL(OLD.LastSortSection, '') AND 
        Type = 0 AND 
        number <= 0; 
END;
CREATE TRIGGER FirstSortSectionTotalInsert AFTER INSERT ON ABPerson 
WHEN NEW.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO FirstSortSectionCountTotal 
    VALUES( 
        NEW.FirstSortLanguageIndex, 
        IFNULL(NEW.FirstSortSection, ''), 
        0, 
        1 + IFNULL((SELECT number from FirstSortSectionCountTotal WHERE LanguageIndex = NEW.FirstSortLanguageIndex AND Section = IFNULL(NEW.FirstSortSection, '') AND Type = 0), 0) 
    ); 
END;
CREATE TRIGGER LastSortSectionTotalInsert AFTER INSERT ON ABPerson 
WHEN NEW.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO LastSortSectionCountTotal 
    VALUES( 
        NEW.LastSortLanguageIndex, 
        IFNULL(NEW.LastSortSection, ''), 
        0, 
        1 + IFNULL((SELECT number from LastSortSectionCountTotal WHERE LanguageIndex = NEW.LastSortLanguageIndex AND Section = IFNULL(NEW.LastSortSection, '') AND Type = 0), 0) 
    ); 
END;
CREATE TRIGGER FirstSortSectionTotalUpdateIfWasPreferred AFTER UPDATE ON ABPerson 
WHEN OLD.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO FirstSortSectionCountTotal 
    VALUES( 
        OLD.FirstSortLanguageIndex, 
        IFNULL(OLD.FirstSortSection, ''), 
        0, 
        (SELECT number FROM FirstSortSectionCountTotal WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND Section = IFNULL(OLD.FirstSortSection, '') AND Type = 0) - 1 
    ); 
    
    DELETE FROM FirstSortSectionCountTotal 
    WHERE LanguageIndex = OLD.FirstSortLanguageIndex AND 
        Section = IFNULL(OLD.FirstSortSection, '') AND 
        Type = 0 AND 
        number = 0; 
END;
CREATE TRIGGER FirstSortSectionTotalUpdateIfIsPreferred AFTER UPDATE ON ABPerson 
WHEN NEW.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO FirstSortSectionCountTotal 
    VALUES( 
        NEW.FirstSortLanguageIndex, 
        IFNULL(NEW.FirstSortSection, ''), 
        0, 
        1 + IFNULL((SELECT number from FirstSortSectionCountTotal WHERE LanguageIndex = NEW.FirstSortLanguageIndex AND Section = IFNULL(NEW.FirstSortSection, '') AND Type = 0), 0) 
    ); 
END;
CREATE TRIGGER LastSortSectionTotalUpdateIfWasPreferred AFTER UPDATE ON ABPerson 
WHEN OLD.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO LastSortSectionCountTotal 
    VALUES( 
        OLD.LastSortLanguageIndex, 
        IFNULL(OLD.LastSortSection, ''), 
        0, 
        (SELECT number FROM LastSortSectionCountTotal WHERE LanguageIndex = OLD.LastSortLanguageIndex AND Section = IFNULL(OLD.LastSortSection, '') AND Type = 0) - 1 
    ); 
    
    DELETE FROM LastSortSectionCountTotal 
    WHERE LanguageIndex = OLD.LastSortLanguageIndex AND 
        Section = IFNULL(OLD.LastSortSection, '') AND 
        Type = 0 AND 
        number = 0; 
END;
CREATE TRIGGER LastSortSectionTotalUpdateIfIsPreferred AFTER UPDATE ON ABPerson 
WHEN NEW.IsPreferredName=1 
BEGIN 
    INSERT OR REPLACE INTO LastSortSectionCountTotal 
    VALUES( 
        NEW.LastSortLanguageIndex, 
        IFNULL(NEW.LastSortSection, ''), 
        0, 
        1 + IFNULL((SELECT number from LastSortSectionCountTotal WHERE LanguageIndex = NEW.LastSortLanguageIndex AND Section = IFNULL(NEW.LastSortSection, '') AND Type = 0), 0) 
    ); 
END;
CREATE VIRTUAL TABLE ABPersonFullTextSearch USING fts4(tokenize=ab_cf_tokenizer language="en" collator="en@collation=search", First, Last, Middle, FirstPhonetic, MiddlePhonetic, LastPhonetic, Organization, OrganizationPhonetic, Department, Note, Birthday, JobTitle, Nickname, Prefix, Suffix, DisplayName, Phone, Email, Address, SocialProfile, URL, RelatedNames, IM, Date, SupplementalCompositeNameTerms);
CREATE VIRTUAL TABLE ABPersonSmartDialerFullTextSearch USING fts4(tokenize=ab_cf_tokenizer language="en" collator="en@collation=search", FirstEncoding, MiddleEncoding, LastEncoding, OrganizationEncoding, NicknameEncoding, PhoneLastFourDigits);
