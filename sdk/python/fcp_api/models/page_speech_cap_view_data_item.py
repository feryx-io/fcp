from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.cap_unit import CapUnit
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageSpeechCapViewDataItem")



@_attrs_define
class PageSpeechCapViewDataItem:
    """ One provider's cap of one unit on the tenant this month.

        Attributes:
            provider (str): A platform provider's name, or `account:<uuid>`.
            label (str): The provider, or the account's name.
            unit (CapUnit): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
                provider synthesized, or minutes a recognizer heard (counted in
                milliseconds).
            platform_override (bool): Whether the platform set a cap for this tenant itself, rather than
                its default (null `platform_cap` then means none for this tenant).
            used (int): Used this month (UTC), the cloud's and sites': characters, or
                whole minutes heard.
            state (str): `ok`, `near` (80 % or more) or `reached`.
            resets_at (str): When the month's count starts again (RFC 3339).
            platform_cap (int | None | Unset): What the platform holds the tenant to on this provider (its cap
                for this tenant, else its default); never on an account.
            own_cap (int | None | Unset): The tenant's own cap.
            cap (int | None | Unset): The cap that applies: the lowest of the two.
     """

    provider: str
    label: str
    unit: CapUnit
    platform_override: bool
    used: int
    state: str
    resets_at: str
    platform_cap: int | None | Unset = UNSET
    own_cap: int | None | Unset = UNSET
    cap: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        provider = self.provider

        label = self.label

        unit = self.unit.value

        platform_override = self.platform_override

        used = self.used

        state = self.state

        resets_at = self.resets_at

        platform_cap: int | None | Unset
        if isinstance(self.platform_cap, Unset):
            platform_cap = UNSET
        else:
            platform_cap = self.platform_cap

        own_cap: int | None | Unset
        if isinstance(self.own_cap, Unset):
            own_cap = UNSET
        else:
            own_cap = self.own_cap

        cap: int | None | Unset
        if isinstance(self.cap, Unset):
            cap = UNSET
        else:
            cap = self.cap


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "provider": provider,
            "label": label,
            "unit": unit,
            "platform_override": platform_override,
            "used": used,
            "state": state,
            "resets_at": resets_at,
        })
        if platform_cap is not UNSET:
            field_dict["platform_cap"] = platform_cap
        if own_cap is not UNSET:
            field_dict["own_cap"] = own_cap
        if cap is not UNSET:
            field_dict["cap"] = cap

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        provider = d.pop("provider")

        label = d.pop("label")

        unit = CapUnit(d.pop("unit"))




        platform_override = d.pop("platform_override")

        used = d.pop("used")

        state = d.pop("state")

        resets_at = d.pop("resets_at")

        def _parse_platform_cap(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        platform_cap = _parse_platform_cap(d.pop("platform_cap", UNSET))


        def _parse_own_cap(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        own_cap = _parse_own_cap(d.pop("own_cap", UNSET))


        def _parse_cap(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        cap = _parse_cap(d.pop("cap", UNSET))


        page_speech_cap_view_data_item = cls(
            provider=provider,
            label=label,
            unit=unit,
            platform_override=platform_override,
            used=used,
            state=state,
            resets_at=resets_at,
            platform_cap=platform_cap,
            own_cap=own_cap,
            cap=cap,
        )


        page_speech_cap_view_data_item.additional_properties = d
        return page_speech_cap_view_data_item

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
