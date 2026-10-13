from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.destination_override import DestinationOverride





T = TypeVar("T", bound="TenantCalling")



@_attrs_define
class TenantCalling:
    """ A tenant's calling defaults.

        Attributes:
            country (None | str | Unset): ISO 3166-1 alpha-2 (`US`, `GB`, …), for classifying the numbers the
                tenant's users dial; unset reads numbers as North American.
            timezone (None | str | Unset): IANA timezone, the default for the tenant's new schedules.
            destination_overrides (list[DestinationOverride] | Unset): The tenant's own classification of number ranges,
                over the
                built-in table.
            hoteling_session_hours (int | None | Unset): How long a hoteling guest stays logged in on a host phone
                (`docs/ENTERPRISE_CALLING.md` §3.8); unset is
                [`crate::hoteling::DEFAULT_SESSION_HOURS`].
            hold_music (None | str | Unset): The tenant's hold music (`docs/ENTERPRISE_CALLING.md` §3.3.1): a
                prompt of the tenant, or [`HOLD_MUSIC_NONE`](crate::calling::HOLD_MUSIC_NONE)
                for silence; unset is the tenant's first hold-music prompt, else
                the node's built-in music. A holder's or a number's own choice
                comes first.
     """

    country: None | str | Unset = UNSET
    timezone: None | str | Unset = UNSET
    destination_overrides: list[DestinationOverride] | Unset = UNSET
    hoteling_session_hours: int | None | Unset = UNSET
    hold_music: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.destination_override import DestinationOverride # noqa: PLC0415
        country: None | str | Unset
        if isinstance(self.country, Unset):
            country = UNSET
        else:
            country = self.country

        timezone: None | str | Unset
        if isinstance(self.timezone, Unset):
            timezone = UNSET
        else:
            timezone = self.timezone

        destination_overrides: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.destination_overrides, Unset):
            destination_overrides = []
            for destination_overrides_item_data in self.destination_overrides:
                destination_overrides_item = destination_overrides_item_data.to_dict()
                destination_overrides.append(destination_overrides_item)



        hoteling_session_hours: int | None | Unset
        if isinstance(self.hoteling_session_hours, Unset):
            hoteling_session_hours = UNSET
        else:
            hoteling_session_hours = self.hoteling_session_hours

        hold_music: None | str | Unset
        if isinstance(self.hold_music, Unset):
            hold_music = UNSET
        else:
            hold_music = self.hold_music


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if country is not UNSET:
            field_dict["country"] = country
        if timezone is not UNSET:
            field_dict["timezone"] = timezone
        if destination_overrides is not UNSET:
            field_dict["destination_overrides"] = destination_overrides
        if hoteling_session_hours is not UNSET:
            field_dict["hoteling_session_hours"] = hoteling_session_hours
        if hold_music is not UNSET:
            field_dict["hold_music"] = hold_music

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.destination_override import DestinationOverride # noqa: PLC0415
        d = dict(src_dict)
        def _parse_country(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        country = _parse_country(d.pop("country", UNSET))


        def _parse_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        timezone = _parse_timezone(d.pop("timezone", UNSET))


        _destination_overrides = d.pop("destination_overrides", UNSET)
        destination_overrides: list[DestinationOverride] | Unset = UNSET
        if _destination_overrides is not UNSET:
            destination_overrides = []
            for destination_overrides_item_data in _destination_overrides:
                destination_overrides_item = DestinationOverride.from_dict(destination_overrides_item_data)



                destination_overrides.append(destination_overrides_item)


        def _parse_hoteling_session_hours(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        hoteling_session_hours = _parse_hoteling_session_hours(d.pop("hoteling_session_hours", UNSET))


        def _parse_hold_music(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        hold_music = _parse_hold_music(d.pop("hold_music", UNSET))


        tenant_calling = cls(
            country=country,
            timezone=timezone,
            destination_overrides=destination_overrides,
            hoteling_session_hours=hoteling_session_hours,
            hold_music=hold_music,
        )


        tenant_calling.additional_properties = d
        return tenant_calling

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
