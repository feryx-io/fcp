from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field
import json
from .. import types

from ..types import UNSET, Unset

from ..types import File, FileTypes
from io import BytesIO






T = TypeVar("T", bound="GreetingUpload")



@_attrs_define
class GreetingUpload:
    """ `POST /voicemail/mailboxes/:id/greetings`.

        Attributes:
            greeting_type (str): `unavailable`, `busy`, `name` or `temporary`.
            audio (File): The recording: a WAV file.
     """

    greeting_type: str
    audio: File
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        greeting_type = self.greeting_type

        audio = self.audio.to_tuple()



        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "greeting_type": greeting_type,
            "audio": audio,
        })

        return field_dict


    def to_multipart(self) -> types.RequestFiles:
        files: types.RequestFiles = []

        files.append(("greeting_type", (None, str(self.greeting_type).encode(), "text/plain")))



        files.append(("audio", self.audio.to_tuple()))




        for prop_name, prop in self.additional_properties.items():
            files.append((prop_name, (None, str(prop).encode(), "text/plain")))



        return files


    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        greeting_type = d.pop("greeting_type")

        audio = File(
             payload = BytesIO(d.pop("audio"))
        )




        greeting_upload = cls(
            greeting_type=greeting_type,
            audio=audio,
        )


        greeting_upload.additional_properties = d
        return greeting_upload

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
