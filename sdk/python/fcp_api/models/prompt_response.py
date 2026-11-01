from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PromptResponse")



@_attrs_define
class PromptResponse:
    """ 
        Attributes:
            id (str):
            name (str):
            category (str):
            language (str):
            source_type (str):
            is_system (bool):
            has_audio (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            tenant_id (None | str | Unset): The owning tenant; `null` for a system prompt, the platform's.
            description (None | str | Unset):
            tts_text (None | str | Unset):
            tts_voice (None | str | Unset):
            audio_path (None | str | Unset):
            audio_format (None | str | Unset):
            duration_ms (int | None | Unset):
            file_size (int | None | Unset):
            overridden_by (None | str | Unset): On a system prompt: the id of the tenant's override of it, which the
                tenant's calls hear instead.
            overrides (None | str | Unset): On a tenant's prompt: the id of the system prompt it overrides.
     """

    id: str
    name: str
    category: str
    language: str
    source_type: str
    is_system: bool
    has_audio: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    tenant_id: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    tts_text: None | str | Unset = UNSET
    tts_voice: None | str | Unset = UNSET
    audio_path: None | str | Unset = UNSET
    audio_format: None | str | Unset = UNSET
    duration_ms: int | None | Unset = UNSET
    file_size: int | None | Unset = UNSET
    overridden_by: None | str | Unset = UNSET
    overrides: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        category = self.category

        language = self.language

        source_type = self.source_type

        is_system = self.is_system

        has_audio = self.has_audio

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        tts_text: None | str | Unset
        if isinstance(self.tts_text, Unset):
            tts_text = UNSET
        else:
            tts_text = self.tts_text

        tts_voice: None | str | Unset
        if isinstance(self.tts_voice, Unset):
            tts_voice = UNSET
        else:
            tts_voice = self.tts_voice

        audio_path: None | str | Unset
        if isinstance(self.audio_path, Unset):
            audio_path = UNSET
        else:
            audio_path = self.audio_path

        audio_format: None | str | Unset
        if isinstance(self.audio_format, Unset):
            audio_format = UNSET
        else:
            audio_format = self.audio_format

        duration_ms: int | None | Unset
        if isinstance(self.duration_ms, Unset):
            duration_ms = UNSET
        else:
            duration_ms = self.duration_ms

        file_size: int | None | Unset
        if isinstance(self.file_size, Unset):
            file_size = UNSET
        else:
            file_size = self.file_size

        overridden_by: None | str | Unset
        if isinstance(self.overridden_by, Unset):
            overridden_by = UNSET
        else:
            overridden_by = self.overridden_by

        overrides: None | str | Unset
        if isinstance(self.overrides, Unset):
            overrides = UNSET
        else:
            overrides = self.overrides


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "category": category,
            "language": language,
            "source_type": source_type,
            "is_system": is_system,
            "has_audio": has_audio,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if description is not UNSET:
            field_dict["description"] = description
        if tts_text is not UNSET:
            field_dict["tts_text"] = tts_text
        if tts_voice is not UNSET:
            field_dict["tts_voice"] = tts_voice
        if audio_path is not UNSET:
            field_dict["audio_path"] = audio_path
        if audio_format is not UNSET:
            field_dict["audio_format"] = audio_format
        if duration_ms is not UNSET:
            field_dict["duration_ms"] = duration_ms
        if file_size is not UNSET:
            field_dict["file_size"] = file_size
        if overridden_by is not UNSET:
            field_dict["overridden_by"] = overridden_by
        if overrides is not UNSET:
            field_dict["overrides"] = overrides

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        category = d.pop("category")

        language = d.pop("language")

        source_type = d.pop("source_type")

        is_system = d.pop("is_system")

        has_audio = d.pop("has_audio")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_tts_text(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tts_text = _parse_tts_text(d.pop("tts_text", UNSET))


        def _parse_tts_voice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tts_voice = _parse_tts_voice(d.pop("tts_voice", UNSET))


        def _parse_audio_path(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        audio_path = _parse_audio_path(d.pop("audio_path", UNSET))


        def _parse_audio_format(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        audio_format = _parse_audio_format(d.pop("audio_format", UNSET))


        def _parse_duration_ms(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_ms = _parse_duration_ms(d.pop("duration_ms", UNSET))


        def _parse_file_size(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        file_size = _parse_file_size(d.pop("file_size", UNSET))


        def _parse_overridden_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        overridden_by = _parse_overridden_by(d.pop("overridden_by", UNSET))


        def _parse_overrides(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        overrides = _parse_overrides(d.pop("overrides", UNSET))


        prompt_response = cls(
            id=id,
            name=name,
            category=category,
            language=language,
            source_type=source_type,
            is_system=is_system,
            has_audio=has_audio,
            created_at=created_at,
            updated_at=updated_at,
            tenant_id=tenant_id,
            description=description,
            tts_text=tts_text,
            tts_voice=tts_voice,
            audio_path=audio_path,
            audio_format=audio_format,
            duration_ms=duration_ms,
            file_size=file_size,
            overridden_by=overridden_by,
            overrides=overrides,
        )


        prompt_response.additional_properties = d
        return prompt_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
