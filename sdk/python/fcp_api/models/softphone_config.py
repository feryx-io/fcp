from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.softphone_account import SoftphoneAccount
  from ..models.softphone_config_feature_codes import SoftphoneConfigFeatureCodes
  from ..models.softphone_line import SoftphoneLine
  from ..models.softphone_media import SoftphoneMedia





T = TypeVar("T", bound="SoftphoneConfig")



@_attrs_define
class SoftphoneConfig:
    """ Everything the app needs.

        Attributes:
            account (SoftphoneAccount): The SIP account the app registers with.
            media (SoftphoneMedia): The media the app offers.
            username (str): The user as FCP knows them.
            feature_codes (SoftphoneConfigFeatureCodes): The feature codes, by name (`park`, `voicemail`, `dnd_on`, …).
            device_id (str): The device the app is.
            may_record (bool): The user may record a call they are on
                (`/me/calls/{call_id}/recording/start`); the app shows its record
                button only then.
            directory_listed (bool): Listed in the tenant's directory, and whether the user may change
                that themselves (`directory_listed` on `PUT /me/profile`).
            may_change_directory_listing (bool):
            lines (list[SoftphoneLine]): The app's lines, its own first; `account` is the first, kept for a
                release.
            may_rename_lines (bool): Whether the user may name their lines (the tenant's choice).
            extension (None | str | Unset):
            api_url (None | str | Unset): Where this admin is, for the app's API calls.
     """

    account: SoftphoneAccount
    media: SoftphoneMedia
    username: str
    feature_codes: SoftphoneConfigFeatureCodes
    device_id: str
    may_record: bool
    directory_listed: bool
    may_change_directory_listing: bool
    lines: list[SoftphoneLine]
    may_rename_lines: bool
    extension: None | str | Unset = UNSET
    api_url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.softphone_account import SoftphoneAccount # noqa: PLC0415
        from ..models.softphone_config_feature_codes import SoftphoneConfigFeatureCodes # noqa: PLC0415
        from ..models.softphone_line import SoftphoneLine # noqa: PLC0415
        from ..models.softphone_media import SoftphoneMedia # noqa: PLC0415
        account = self.account.to_dict()

        media = self.media.to_dict()

        username = self.username

        feature_codes = self.feature_codes.to_dict()

        device_id = self.device_id

        may_record = self.may_record

        directory_listed = self.directory_listed

        may_change_directory_listing = self.may_change_directory_listing

        lines = []
        for lines_item_data in self.lines:
            lines_item = lines_item_data.to_dict()
            lines.append(lines_item)



        may_rename_lines = self.may_rename_lines

        extension: None | str | Unset
        if isinstance(self.extension, Unset):
            extension = UNSET
        else:
            extension = self.extension

        api_url: None | str | Unset
        if isinstance(self.api_url, Unset):
            api_url = UNSET
        else:
            api_url = self.api_url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "account": account,
            "media": media,
            "username": username,
            "feature_codes": feature_codes,
            "device_id": device_id,
            "may_record": may_record,
            "directory_listed": directory_listed,
            "may_change_directory_listing": may_change_directory_listing,
            "lines": lines,
            "may_rename_lines": may_rename_lines,
        })
        if extension is not UNSET:
            field_dict["extension"] = extension
        if api_url is not UNSET:
            field_dict["api_url"] = api_url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.softphone_account import SoftphoneAccount # noqa: PLC0415
        from ..models.softphone_config_feature_codes import SoftphoneConfigFeatureCodes # noqa: PLC0415
        from ..models.softphone_line import SoftphoneLine # noqa: PLC0415
        from ..models.softphone_media import SoftphoneMedia # noqa: PLC0415
        d = dict(src_dict)
        account = SoftphoneAccount.from_dict(d.pop("account"))




        media = SoftphoneMedia.from_dict(d.pop("media"))




        username = d.pop("username")

        feature_codes = SoftphoneConfigFeatureCodes.from_dict(d.pop("feature_codes"))




        device_id = d.pop("device_id")

        may_record = d.pop("may_record")

        directory_listed = d.pop("directory_listed")

        may_change_directory_listing = d.pop("may_change_directory_listing")

        lines = []
        _lines = d.pop("lines")
        for lines_item_data in (_lines):
            lines_item = SoftphoneLine.from_dict(lines_item_data)



            lines.append(lines_item)


        may_rename_lines = d.pop("may_rename_lines")

        def _parse_extension(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        extension = _parse_extension(d.pop("extension", UNSET))


        def _parse_api_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        api_url = _parse_api_url(d.pop("api_url", UNSET))


        softphone_config = cls(
            account=account,
            media=media,
            username=username,
            feature_codes=feature_codes,
            device_id=device_id,
            may_record=may_record,
            directory_listed=directory_listed,
            may_change_directory_listing=may_change_directory_listing,
            lines=lines,
            may_rename_lines=may_rename_lines,
            extension=extension,
            api_url=api_url,
        )


        softphone_config.additional_properties = d
        return softphone_config

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
