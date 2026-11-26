from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.speech_settings_response_voices import SpeechSettingsResponseVoices





T = TypeVar("T", bound="SpeechSettingsResponse")



@_attrs_define
class SpeechSettingsResponse:
    """ The tenant's speech settings; nothing set is the platform's defaults.

        Attributes:
            tenant_id (str):
            voices (SpeechSettingsResponseVoices): A voice per language (`es` or `es-ES`), over `voice`.
            configured (bool): False until the tenant saved settings.
            updated_at (datetime.datetime):
            provider (None | str | Unset): One of the TTS server's providers; null is its default.
            voice (None | str | Unset): The voice where no language below names one; null is the provider's.
            language (None | str | Unset): What a request without a language is spoken in; null is `en-US`.
     """

    tenant_id: str
    voices: SpeechSettingsResponseVoices
    configured: bool
    updated_at: datetime.datetime
    provider: None | str | Unset = UNSET
    voice: None | str | Unset = UNSET
    language: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.speech_settings_response_voices import SpeechSettingsResponseVoices # noqa: PLC0415
        tenant_id = self.tenant_id

        voices = self.voices.to_dict()

        configured = self.configured

        updated_at = self.updated_at.isoformat()

        provider: None | str | Unset
        if isinstance(self.provider, Unset):
            provider = UNSET
        else:
            provider = self.provider

        voice: None | str | Unset
        if isinstance(self.voice, Unset):
            voice = UNSET
        else:
            voice = self.voice

        language: None | str | Unset
        if isinstance(self.language, Unset):
            language = UNSET
        else:
            language = self.language


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "voices": voices,
            "configured": configured,
            "updated_at": updated_at,
        })
        if provider is not UNSET:
            field_dict["provider"] = provider
        if voice is not UNSET:
            field_dict["voice"] = voice
        if language is not UNSET:
            field_dict["language"] = language

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.speech_settings_response_voices import SpeechSettingsResponseVoices # noqa: PLC0415
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        voices = SpeechSettingsResponseVoices.from_dict(d.pop("voices"))




        configured = d.pop("configured")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_provider(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provider = _parse_provider(d.pop("provider", UNSET))


        def _parse_voice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        voice = _parse_voice(d.pop("voice", UNSET))


        def _parse_language(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        language = _parse_language(d.pop("language", UNSET))


        speech_settings_response = cls(
            tenant_id=tenant_id,
            voices=voices,
            configured=configured,
            updated_at=updated_at,
            provider=provider,
            voice=voice,
            language=language,
        )


        speech_settings_response.additional_properties = d
        return speech_settings_response

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
