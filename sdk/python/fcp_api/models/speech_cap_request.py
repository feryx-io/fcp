from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SpeechCapRequest")



@_attrs_define
class SpeechCapRequest:
    """ A cap to keep. `monthly_characters` null is allowed only on a tenant's
    platform cap: none for this tenant, whatever the default says.

        Attributes:
            monthly_characters (int | None | Unset):
            kind (None | str | Unset): `tenant` (the default: the tenant's own) or `platform` (a platform
                administrator's cap on this tenant).
     """

    monthly_characters: int | None | Unset = UNSET
    kind: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        monthly_characters: int | None | Unset
        if isinstance(self.monthly_characters, Unset):
            monthly_characters = UNSET
        else:
            monthly_characters = self.monthly_characters

        kind: None | str | Unset
        if isinstance(self.kind, Unset):
            kind = UNSET
        else:
            kind = self.kind


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if monthly_characters is not UNSET:
            field_dict["monthly_characters"] = monthly_characters
        if kind is not UNSET:
            field_dict["kind"] = kind

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_monthly_characters(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        monthly_characters = _parse_monthly_characters(d.pop("monthly_characters", UNSET))


        def _parse_kind(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kind = _parse_kind(d.pop("kind", UNSET))


        speech_cap_request = cls(
            monthly_characters=monthly_characters,
            kind=kind,
        )

        return speech_cap_request

